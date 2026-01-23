//! # PetraCache
//!
//! High-performance memcached-compatible cache server backed by RocksDB.
//!
//! *Petra* (πέτρα) means "rock" in Greek - a nod to the RocksDB storage engine.
//!
//! ## Features
//!
//! - Memcached ASCII protocol support (GET, SET, DELETE, VERSION)
//! - RocksDB persistent storage with configurable options
//! - TTL support with lazy expiration and compaction filter
//! - Prometheus metrics endpoint
//! - Health check endpoints for load balancer integration
//! - Designed to work behind mcrouter for routing and failover
//!
//! ## Example
//!
//! ```ignore
//! use petracache::config::Config;
//! use petracache::storage::RocksStorage;
//! use petracache::server::Server;
//!
//! let config = Config::default();
//! let storage = RocksStorage::open(&config.storage)?;
//! ```
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────┐     ┌───────────┐     ┌─────────────────────────┐
//! │ app/service  │────▶│ mcrouter  │────▶│ PetraCache              │
//! │ (memcache    │     │ (routing, │     │  ├─ ASCII protocol      │
//! │  client)     │     │  failover)│     │  ├─ TTL support         │
//! └──────────────┘     └───────────┘     │  └─ RocksDB backend     │
//!                                        └─────────────────────────┘
//! ```

// Modules
pub mod config;
pub mod error;
pub mod health;
pub mod metrics;
pub mod prelude;
pub mod protocol;
pub mod server;
pub mod storage;

// Re-exports for convenience
pub use error::{PetraCacheError, ProtocolError, Result, StorageError};
