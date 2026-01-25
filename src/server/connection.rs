//! Connection handling for individual client connections

use super::Server;
use super::handler;
use crate::protocol::{
    Command, ParseResult, PendingStorageCommand, ResponseWriter, parse, parse_storage_command_line,
    parse_storage_data,
};
use bytes::BytesMut;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::OwnedSemaphorePermit;
use tracing::debug;

/// Handle a single client connection
pub async fn handle(
    server: Arc<Server>,
    mut stream: TcpStream,
    _permit: OwnedSemaphorePermit,
) -> anyhow::Result<()> {
    let mut read_buf = BytesMut::with_capacity(server.config.read_buffer_size);
    let mut response = ResponseWriter::new(server.config.write_buffer_size);
    let mut pending_storage: Option<PendingStorageCommand> = None;

    loop {
        tokio::select! {
            _ = server.cancel_token.cancelled() => {
                break;
            }
            result = stream.read_buf(&mut read_buf) => {
                match result {
                    Ok(0) => {
                        // Connection closed
                        break;
                    }
                    Ok(n) => {
                        server.metrics.bytes_read.inc_by(n as u64);

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

                                    let should_quit = matches!(cmd, Command::Quit);
                                    let noreply = cmd.is_noreply();

                                    // Execute command
                                    handler::execute(&server, cmd, &mut response);

                                    // Consume processed bytes
                                    let _ = read_buf.split_to(consumed);

                                    // Send response if not noreply
                                    if !noreply && !response.is_empty() {
                                        let buf = response.take();
                                        server.metrics.bytes_written.inc_by(buf.len() as u64);
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
                                    server.metrics.protocol_errors.inc();
                                    response.client_error(&e.to_string());

                                    // Try to recover by finding next command
                                    if let Some(pos) = find_crlf(&read_buf) {
                                        let _ = read_buf.split_to(pos + 2);
                                    } else {
                                        read_buf.clear();
                                    }
                                    pending_storage = None;

                                    let buf = response.take();
                                    server.metrics.bytes_written.inc_by(buf.len() as u64);
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

    server.metrics.active_connections.dec();
    Ok(())
}

/// Find \r\n in buffer using SIMD-accelerated search
#[inline]
fn find_crlf(buf: &[u8]) -> Option<usize> {
    memchr::memchr(b'\r', buf).filter(|&i| buf.get(i + 1) == Some(&b'\n'))
}
