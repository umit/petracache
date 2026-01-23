//! Main TCP server for memcached protocol

mod connection;
mod handler;

use crate::config::ServerConfig;
use crate::metrics::Metrics;
use crate::storage::RocksStorage;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Main server struct
pub struct Server {
    pub(crate) config: ServerConfig,
    pub(crate) storage: Arc<RocksStorage>,
    pub(crate) metrics: Arc<Metrics>,
    connection_semaphore: Arc<Semaphore>,
    pub(crate) cancel_token: CancellationToken,
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
                                        if let Err(e) = connection::handle(server, stream, permit).await {
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
}
