//! Configuration for PetraCache

use serde::Deserialize;
use std::path::PathBuf;

/// Main configuration structure
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub metrics: MetricsConfig,
}

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address to listen on
    pub listen_addr: String,

    /// Maximum number of concurrent connections
    pub max_connections: usize,

    /// Read buffer size per connection (bytes)
    pub read_buffer_size: usize,

    /// Write buffer size per connection (bytes)
    pub write_buffer_size: usize,

    /// Number of Tokio worker threads (0 = number of CPUs)
    pub worker_threads: usize,

    /// Connection timeout in seconds (0 = no timeout)
    pub connection_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:11211".to_string(),
            max_connections: 10000,
            read_buffer_size: 8192,
            write_buffer_size: 8192,
            worker_threads: 0,
            connection_timeout_secs: 0,
        }
    }
}

/// Storage (RocksDB) configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Path to RocksDB data directory
    pub db_path: PathBuf,

    /// Block cache size in bytes (1GB default for in-memory performance)
    pub block_cache_size: usize,

    /// Write buffer size in bytes
    pub write_buffer_size: usize,

    /// Maximum number of write buffers
    pub max_write_buffer_number: i32,

    /// Target file size for level-1 in bytes
    pub target_file_size_base: u64,

    /// Maximum number of background jobs
    pub max_background_jobs: i32,

    /// Enable compression
    pub enable_compression: bool,

    /// Enable TTL compaction filter (runs during RocksDB compaction)
    pub enable_ttl_compaction: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("./data/rocksdb"),
            block_cache_size: 1024 * 1024 * 1024, // 1GB block cache
            write_buffer_size: 64 * 1024 * 1024,  // 64MB
            max_write_buffer_number: 3,
            target_file_size_base: 64 * 1024 * 1024, // 64MB
            max_background_jobs: 4,
            enable_compression: false,
            enable_ttl_compaction: true,
        }
    }
}

/// Metrics and health check configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    /// Enable metrics collection
    pub enabled: bool,

    /// Address for metrics/health HTTP server
    pub listen_addr: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_addr: "127.0.0.1:9090".to_string(),
        }
    }
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file(path: &str) -> crate::Result<Self> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            crate::PetraCacheError::Config(format!("Failed to read config file: {e}"))
        })?;

        toml::from_str(&contents)
            .map_err(|e| crate::PetraCacheError::Config(format!("Failed to parse config: {e}")))
    }

    /// Load configuration from environment variables or use defaults
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(addr) = std::env::var("PETRACACHE_LISTEN_ADDR") {
            config.server.listen_addr = addr;
        }

        if let Ok(max_conn) = std::env::var("PETRACACHE_MAX_CONNECTIONS")
            && let Ok(n) = max_conn.parse()
        {
            config.server.max_connections = n;
        }

        if let Ok(path) = std::env::var("PETRACACHE_DB_PATH") {
            config.storage.db_path = PathBuf::from(path);
        }

        if let Ok(addr) = std::env::var("PETRACACHE_METRICS_ADDR") {
            config.metrics.listen_addr = addr;
        }

        if let Ok(enabled) = std::env::var("PETRACACHE_METRICS_ENABLED") {
            config.metrics.enabled = enabled.to_lowercase() == "true" || enabled == "1";
        }

        config
    }
}
