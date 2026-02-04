//! PetraCache - High-performance memcached-compatible cache server
//!
//! A memcached ASCII protocol compatible server backed by RocksDB storage.
//! Designed to work behind mcrouter for routing and failover.

// Use jemalloc for better multi-threaded performance (10-30% throughput improvement)
#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use petracache::config::Config;
use petracache::health::HealthServer;
use petracache::metrics::Metrics;
use petracache::server::Server;
use petracache::storage::RocksStorage;
use std::sync::Arc;
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    info!("Starting PetraCache");

    // Load configuration
    let config = if let Some(config_path) = std::env::args().nth(1) {
        info!("Loading configuration from {}", config_path);
        Config::from_file(&config_path)?
    } else {
        info!("Using default configuration (set PETRACACHE_* env vars to customize)");
        Config::from_env()
    };

    info!("Configuration: {:?}", config);

    // Build tokio runtime with configured worker threads
    let mut runtime_builder = Builder::new_multi_thread();
    if config.server.worker_threads > 0 {
        runtime_builder.worker_threads(config.server.worker_threads);
        info!("Using {} worker threads", config.server.worker_threads);
    } else {
        info!("Using default worker threads (auto-detected)");
    }
    let runtime = runtime_builder.enable_all().build()?;

    runtime.block_on(async_main(config))
}

async fn async_main(config: Config) -> anyhow::Result<()> {
    // Create cancellation token for graceful shutdown
    let cancel_token = CancellationToken::new();

    // Initialize storage
    info!("Opening RocksDB at {:?}", config.storage.db_path);
    let storage = Arc::new(
        RocksStorage::open(&config.storage)
            .map_err(|e| anyhow::anyhow!("Failed to open RocksDB: {e}"))?,
    );

    // Initialize metrics
    let metrics = Arc::new(Metrics::new());

    // Start health server in separate thread if enabled
    let health_server = if config.metrics.enabled {
        let health = Arc::new(HealthServer::new(Arc::clone(&metrics)));
        let health_clone = Arc::clone(&health);
        let metrics_config = config.metrics.clone();

        std::thread::spawn(move || {
            if let Err(e) = health_clone.run(&metrics_config) {
                error!("Health server error: {}", e);
            }
        });

        Some(health)
    } else {
        None
    };

    // Create and start main server
    let server = Arc::new(Server::new(
        config.server.clone(),
        Arc::clone(&storage),
        Arc::clone(&metrics),
        cancel_token.clone(),
    ));

    // Mark as ready after initialization
    if let Some(ref health) = health_server {
        health.set_ready(true);
        info!("Server is ready");
    }

    // Setup signal handlers
    let cancel_for_signal = cancel_token.clone();
    let health_for_signal = health_server.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT, shutting down...");
            }
            _ = async {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{signal, SignalKind};
                    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
                    sigterm.recv().await
                }
                #[cfg(not(unix))]
                {
                    std::future::pending::<Option<()>>().await
                }
            } => {
                info!("Received SIGTERM, shutting down...");
            }
        }
        cancel_for_signal.cancel();
        if let Some(health) = health_for_signal {
            health.stop();
        }
    });

    // Run the main server
    if let Err(e) = server.run().await {
        error!("Server error: {}", e);
    }

    info!("PetraCache stopped");
    Ok(())
}
