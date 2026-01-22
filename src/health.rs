//! Simple HTTP health and metrics server (synchronous)

use crate::config::MetricsConfig;
use crate::metrics::Metrics;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info};

/// Health server state
pub struct HealthServer {
    metrics: Arc<Metrics>,
    ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl HealthServer {
    /// Create a new health server
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self {
            metrics,
            ready: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Set the ready state
    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::SeqCst);
    }

    /// Check if the server is ready
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// Stop the server
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Start the health server (blocking, run in separate thread)
    pub fn run(self: Arc<Self>, config: &MetricsConfig) -> std::io::Result<()> {
        let listener = TcpListener::bind(&config.listen_addr)?;
        listener.set_nonblocking(true)?;
        info!("Health server listening on {}", config.listen_addr);

        while self.running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let server = Arc::clone(&self);
                    // Handle in same thread (simple approach)
                    if let Err(e) = server.handle_connection(stream) {
                        error!("Health connection error: {}", e);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection ready, sleep briefly
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    error!("Health server accept error: {}", e);
                }
            }
        }

        info!("Health server stopped");
        Ok(())
    }

    /// Handle a single HTTP connection
    fn handle_connection(&self, mut stream: TcpStream) -> std::io::Result<()> {
        stream.set_nonblocking(false)?;

        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;

        // Parse simple HTTP request: "GET /path HTTP/1.1"
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            return self.send_response(&mut stream, 400, "text/plain", "Bad Request");
        }

        let method = parts[0];
        let path = parts[1];

        if method != "GET" {
            return self.send_response(&mut stream, 405, "text/plain", "Method Not Allowed");
        }

        match path {
            "/health" | "/healthz" => {
                self.send_response(&mut stream, 200, "application/json", r#"{"status":"healthy"}"#)
            }
            "/ready" | "/readyz" => {
                if self.is_ready() {
                    self.send_response(&mut stream, 200, "application/json", r#"{"status":"ready"}"#)
                } else {
                    self.send_response(&mut stream, 503, "application/json", r#"{"status":"not ready"}"#)
                }
            }
            "/metrics" => {
                let metrics = self.metrics.gather();
                self.send_response(&mut stream, 200, "text/plain; version=0.0.4", &metrics)
            }
            _ => {
                self.send_response(&mut stream, 404, "text/plain", "Not Found")
            }
        }
    }

    /// Send HTTP response
    fn send_response(
        &self,
        stream: &mut TcpStream,
        status: u16,
        content_type: &str,
        body: &str,
    ) -> std::io::Result<()> {
        let status_text = match status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            503 => "Service Unavailable",
            _ => "Unknown",
        };

        let response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            status_text,
            content_type,
            body.len(),
            body
        );

        stream.write_all(response.as_bytes())?;
        stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ready_state() {
        let metrics = Arc::new(Metrics::new());
        let server = HealthServer::new(metrics);

        assert!(!server.is_ready());
        server.set_ready(true);
        assert!(server.is_ready());
        server.set_ready(false);
        assert!(!server.is_ready());
    }
}
