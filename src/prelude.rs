//! Prelude module for common imports.
//!
//! This module re-exports commonly used types and traits for convenience.
//!
//! # Usage
//!
//! ```ignore
//! use petracache::prelude::*;
//! ```

// Error types
pub use crate::error::{PetraCacheError, ProtocolError, Result, StorageError};

// Configuration
pub use crate::config::{Config, MetricsConfig, ServerConfig, StorageConfig};

// Storage
pub use crate::storage::{RocksStorage, StoredValue};

// Protocol
pub use crate::protocol::{Command, ParseResult, ResponseWriter};

// Metrics
pub use crate::metrics::Metrics;

// Server
pub use crate::server::Server;

// Common external crates
pub use std::sync::Arc;
pub use tracing::{debug, error, info, trace, warn};
