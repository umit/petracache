//! Main TCP server for memcached protocol

use crate::config::ServerConfig;
use crate::metrics::Metrics;
use crate::protocol::{
    Command, ParseResult, PendingStorageCommand, ResponseWriter, parse, parse_storage_command_line,
    parse_storage_data,
};
use crate::storage::{RocksStorage, StoredValue};
use bytes::BytesMut;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Version string for memcached VERSION command
const VERSION: &str = "rocksproxy 0.1.0";

/// Main server struct
pub struct Server {
    config: ServerConfig,
    storage: Arc<RocksStorage>,
    metrics: Arc<Metrics>,
    connection_semaphore: Arc<Semaphore>,
    cancel_token: CancellationToken,
}

impl Server {
    /// Create a new server
    pub fn new(
        config: ServerConfig,
        storage: Arc<RocksStorage>,
        metrics: Arc<Metrics>,
        cancel_token: CancellationToken,
    ) -> Self {
        let connection_semaphore = Arc::new(Semaphore::new(config.max_connections));

        Self {
            config,
            storage,
            metrics,
            connection_semaphore,
            cancel_token,
        }
    }

    /// Run the server
    pub async fn run(self: Arc<Self>) -> anyhow::Result<()> {
        let addr: SocketAddr = self.config.listen_addr.parse()?;
        let listener = TcpListener::bind(addr).await?;
        info!("Server listening on {}", addr);

        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    info!("Server shutting down");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            // Disable Nagle's algorithm for lower latency
                            if let Err(e) = stream.set_nodelay(true) {
                                warn!("Failed to set TCP_NODELAY: {}", e);
                            }

                            // Try to acquire connection permit
                            match self.connection_semaphore.clone().try_acquire_owned() {
                                Ok(permit) => {
                                    self.metrics.total_connections.inc();
                                    self.metrics.active_connections.inc();
                                    debug!("Accepted connection from {}", peer_addr);

                                    let server = Arc::clone(&self);
                                    tokio::spawn(async move {
                                        if let Err(e) = server.handle_connection(stream, permit).await {
                                            debug!("Connection error: {}", e);
                                        }
                                    });
                                }
                                Err(_) => {
                                    // Connection limit reached
                                    self.metrics.rejected_connections.inc();
                                    warn!("Connection limit reached, rejecting connection from {}", peer_addr);
                                    drop(stream);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle a single connection
    async fn handle_connection(
        &self,
        mut stream: TcpStream,
        _permit: OwnedSemaphorePermit,
    ) -> anyhow::Result<()> {
        let mut read_buf = BytesMut::with_capacity(self.config.read_buffer_size);
        let mut response = ResponseWriter::new(self.config.write_buffer_size);
        let mut pending_storage: Option<PendingStorageCommand> = None;

        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                result = stream.read_buf(&mut read_buf) => {
                    match result {
                        Ok(0) => {
                            // Connection closed
                            break;
                        }
                        Ok(n) => {
                            self.metrics.bytes_read.inc_by(n as u64);

                            // Process all complete commands in the buffer
                            loop {
                                let parse_result = if let Some(ref pending) = pending_storage {
                                    // We're waiting for data block
                                    parse_storage_data(&read_buf, pending)
                                } else {
                                    // Parse new command
                                    parse(&read_buf)
                                };

                                match parse_result {
                                    ParseResult::Complete(cmd, consumed) => {
                                        pending_storage = None;
                                        let start = Instant::now();

                                        let should_quit = matches!(cmd, Command::Quit);
                                        let noreply = cmd.is_noreply();

                                        // Execute command
                                        self.execute_command(cmd, &mut response);

                                        // Record latency
                                        self.metrics.cmd_latency.observe(start.elapsed().as_secs_f64());

                                        // Consume processed bytes
                                        let _ = read_buf.split_to(consumed);

                                        // Send response if not noreply
                                        if !noreply && !response.is_empty() {
                                            let buf = response.take();
                                            self.metrics.bytes_written.inc_by(buf.len() as u64);
                                            stream.write_all(&buf).await?;
                                        }
                                        response.clear();

                                        if should_quit {
                                            return Ok(());
                                        }
                                    }
                                    ParseResult::NeedMoreData => {
                                        // Check if this is a storage command waiting for data
                                        if pending_storage.is_none()
                                            && let Ok(Some(pending)) = parse_storage_command_line(&read_buf)
                                        {
                                            pending_storage = Some(pending);
                                        }
                                        break;
                                    }
                                    ParseResult::Error(e) => {
                                        self.metrics.protocol_errors.inc();
                                        response.client_error(&e.to_string());

                                        // Try to recover by finding next command
                                        if let Some(pos) = find_crlf(&read_buf) {
                                            let _ = read_buf.split_to(pos + 2);
                                        } else {
                                            read_buf.clear();
                                        }
                                        pending_storage = None;

                                        let buf = response.take();
                                        self.metrics.bytes_written.inc_by(buf.len() as u64);
                                        stream.write_all(&buf).await?;
                                        response.clear();
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Read error: {}", e);
                            break;
                        }
                    }
                }
            }
        }

        self.metrics.active_connections.dec();
        Ok(())
    }

    /// Execute a parsed command
    fn execute_command(&self, cmd: Command<'_>, response: &mut ResponseWriter) {
        match cmd {
            Command::Get { keys } => {
                self.metrics.cmd_get.inc();
                self.handle_get(keys, response, false);
            }
            Command::Gets { keys } => {
                self.metrics.cmd_get.inc();
                self.handle_get(keys, response, true);
            }
            Command::Set {
                key,
                flags,
                exptime,
                data,
                ..
            } => {
                self.metrics.cmd_set.inc();
                self.handle_set(&key, flags, exptime, &data, response);
            }
            Command::Delete { key, .. } => {
                self.metrics.cmd_delete.inc();
                self.handle_delete(&key, response);
            }
            Command::Version => {
                response.version(VERSION);
            }
            Command::Stats { .. } => {
                self.handle_stats(response);
            }
            Command::Quit => {
                // Handled in main loop
            }
            // Unsupported commands
            Command::Add { .. }
            | Command::Replace { .. }
            | Command::Incr { .. }
            | Command::Decr { .. }
            | Command::Touch { .. }
            | Command::FlushAll { .. } => {
                response.server_error("Command not supported");
            }
        }
    }

    /// Handle GET/GETS command
    fn handle_get(
        &self,
        keys: Vec<std::borrow::Cow<'_, [u8]>>,
        response: &mut ResponseWriter,
        with_cas: bool,
    ) {
        let keys_vec: Vec<Vec<u8>> = keys.iter().map(|k| k.to_vec()).collect();

        match self.storage.get_multi(&keys_vec) {
            Ok(results) => {
                for (key, value_opt) in results {
                    if let Some(value) = value_opt {
                        self.metrics.get_hits.inc();
                        if with_cas {
                            response.value_with_cas(&key, value.flags, &value.data, 0);
                        } else {
                            response.value(&key, value.flags, &value.data);
                        }
                    } else {
                        self.metrics.get_misses.inc();
                    }
                }
                response.end();
            }
            Err(e) => {
                self.metrics.storage_errors.inc();
                response.server_error(&e.to_string());
            }
        }
    }

    /// Handle SET command
    fn handle_set(
        &self,
        key: &[u8],
        flags: u32,
        exptime: u64,
        data: &[u8],
        response: &mut ResponseWriter,
    ) {
        let value = StoredValue::new(flags, exptime, data.to_vec());
        match self.storage.set(key, value) {
            Ok(()) => response.stored(),
            Err(e) => {
                self.metrics.storage_errors.inc();
                response.server_error(&e.to_string());
            }
        }
    }

    /// Handle DELETE command
    fn handle_delete(&self, key: &[u8], response: &mut ResponseWriter) {
        match self.storage.delete(key) {
            Ok(true) => response.deleted(),
            Ok(false) => response.not_found(),
            Err(e) => {
                self.metrics.storage_errors.inc();
                response.server_error(&e.to_string());
            }
        }
    }

    /// Handle STATS command
    fn handle_stats(&self, response: &mut ResponseWriter) {
        response.stat("version", VERSION);
        response.stat("cmd_get", &self.metrics.cmd_get.get().to_string());
        response.stat("cmd_set", &self.metrics.cmd_set.get().to_string());
        response.stat("get_hits", &self.metrics.get_hits.get().to_string());
        response.stat("get_misses", &self.metrics.get_misses.get().to_string());
        response.stat(
            "curr_connections",
            &self.metrics.active_connections.get().to_string(),
        );
        response.stat(
            "total_connections",
            &self.metrics.total_connections.get().to_string(),
        );
        response.stat("bytes_read", &self.metrics.bytes_read.get().to_string());
        response.stat(
            "bytes_written",
            &self.metrics.bytes_written.get().to_string(),
        );
        response.end();
    }
}

/// Find \r\n in buffer
fn find_crlf(buf: &[u8]) -> Option<usize> {
    (0..buf.len().saturating_sub(1)).find(|&i| buf[i] == b'\r' && buf[i + 1] == b'\n')
}
