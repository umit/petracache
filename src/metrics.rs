//! Prometheus metrics for RocksProxy

use crate::storage::{EXPIRED_KEYS_REMOVED, TTL_COMPACTION_REMOVED};
use prometheus::{Histogram, HistogramOpts, IntCounter, IntGauge, Registry};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global metrics instance
pub struct Metrics {
    pub registry: Registry,

    // Command counters
    pub cmd_get: IntCounter,
    pub cmd_set: IntCounter,
    pub cmd_add: IntCounter,
    pub cmd_replace: IntCounter,
    pub cmd_delete: IntCounter,
    pub cmd_incr: IntCounter,
    pub cmd_decr: IntCounter,
    pub cmd_touch: IntCounter,
    pub cmd_flush: IntCounter,

    // Hit/miss counters
    pub get_hits: IntCounter,
    pub get_misses: IntCounter,

    // Connection metrics
    pub active_connections: IntGauge,
    pub total_connections: IntCounter,
    pub rejected_connections: IntCounter,

    // Bytes counters
    pub bytes_read: IntCounter,
    pub bytes_written: IntCounter,

    // Latency histograms
    pub cmd_latency: Histogram,

    // Error counters
    pub protocol_errors: IntCounter,
    pub storage_errors: IntCounter,
}

impl Metrics {
    /// Create a new metrics instance
    pub fn new() -> Self {
        let registry = Registry::new();

        let cmd_get = IntCounter::new("petracache_cmd_get_total", "Total GET commands").unwrap();
        let cmd_set = IntCounter::new("petracache_cmd_set_total", "Total SET commands").unwrap();
        let cmd_add = IntCounter::new("petracache_cmd_add_total", "Total ADD commands").unwrap();
        let cmd_replace =
            IntCounter::new("petracache_cmd_replace_total", "Total REPLACE commands").unwrap();
        let cmd_delete =
            IntCounter::new("petracache_cmd_delete_total", "Total DELETE commands").unwrap();
        let cmd_incr = IntCounter::new("petracache_cmd_incr_total", "Total INCR commands").unwrap();
        let cmd_decr = IntCounter::new("petracache_cmd_decr_total", "Total DECR commands").unwrap();
        let cmd_touch =
            IntCounter::new("petracache_cmd_touch_total", "Total TOUCH commands").unwrap();
        let cmd_flush =
            IntCounter::new("petracache_cmd_flush_total", "Total FLUSH_ALL commands").unwrap();

        let get_hits = IntCounter::new("petracache_get_hits_total", "Total GET hits").unwrap();
        let get_misses =
            IntCounter::new("petracache_get_misses_total", "Total GET misses").unwrap();

        let active_connections = IntGauge::new(
            "petracache_active_connections",
            "Current active connections",
        )
        .unwrap();
        let total_connections =
            IntCounter::new("petracache_connections_total", "Total connections accepted").unwrap();
        let rejected_connections = IntCounter::new(
            "petracache_rejected_connections_total",
            "Total connections rejected",
        )
        .unwrap();

        let bytes_read =
            IntCounter::new("petracache_bytes_read_total", "Total bytes read").unwrap();
        let bytes_written =
            IntCounter::new("petracache_bytes_written_total", "Total bytes written").unwrap();

        let cmd_latency = Histogram::with_opts(
            HistogramOpts::new(
                "petracache_cmd_latency_seconds",
                "Command latency in seconds",
            )
            .buckets(vec![
                0.0001, 0.0005, 0.001, 0.002, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
            ]),
        )
        .unwrap();

        let protocol_errors =
            IntCounter::new("petracache_protocol_errors_total", "Total protocol errors").unwrap();
        let storage_errors =
            IntCounter::new("petracache_storage_errors_total", "Total storage errors").unwrap();

        // Register all metrics
        registry.register(Box::new(cmd_get.clone())).unwrap();
        registry.register(Box::new(cmd_set.clone())).unwrap();
        registry.register(Box::new(cmd_add.clone())).unwrap();
        registry.register(Box::new(cmd_replace.clone())).unwrap();
        registry.register(Box::new(cmd_delete.clone())).unwrap();
        registry.register(Box::new(cmd_incr.clone())).unwrap();
        registry.register(Box::new(cmd_decr.clone())).unwrap();
        registry.register(Box::new(cmd_touch.clone())).unwrap();
        registry.register(Box::new(cmd_flush.clone())).unwrap();
        registry.register(Box::new(get_hits.clone())).unwrap();
        registry.register(Box::new(get_misses.clone())).unwrap();
        registry
            .register(Box::new(active_connections.clone()))
            .unwrap();
        registry
            .register(Box::new(total_connections.clone()))
            .unwrap();
        registry
            .register(Box::new(rejected_connections.clone()))
            .unwrap();
        registry.register(Box::new(bytes_read.clone())).unwrap();
        registry.register(Box::new(bytes_written.clone())).unwrap();
        registry.register(Box::new(cmd_latency.clone())).unwrap();
        registry
            .register(Box::new(protocol_errors.clone()))
            .unwrap();
        registry.register(Box::new(storage_errors.clone())).unwrap();

        Self {
            registry,
            cmd_get,
            cmd_set,
            cmd_add,
            cmd_replace,
            cmd_delete,
            cmd_incr,
            cmd_decr,
            cmd_touch,
            cmd_flush,
            get_hits,
            get_misses,
            active_connections,
            total_connections,
            rejected_connections,
            bytes_read,
            bytes_written,
            cmd_latency,
            protocol_errors,
            storage_errors,
        }
    }

    /// Get Prometheus formatted metrics
    pub fn gather(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        let mut output = String::from_utf8(buffer).unwrap();

        // Add TTL expiration stats (from static counters)
        let expired_removed = EXPIRED_KEYS_REMOVED.load(Ordering::Relaxed);
        let compaction_removed = TTL_COMPACTION_REMOVED.load(Ordering::Relaxed);

        output.push_str(&format!(
            "\n# HELP petracache_expired_keys_removed_total Keys removed by lazy expiration or background scan\n\
             # TYPE petracache_expired_keys_removed_total counter\n\
             petracache_expired_keys_removed_total {expired_removed}\n"
        ));

        output.push_str(&format!(
            "\n# HELP petracache_ttl_compaction_removed_total Keys removed by TTL compaction filter\n\
             # TYPE petracache_ttl_compaction_removed_total counter\n\
             petracache_ttl_compaction_removed_total {compaction_removed}\n"
        ));

        output
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Lightweight atomic counters for hot path (used when Prometheus overhead is too high)
pub struct AtomicCounters {
    pub cmd_get: AtomicU64,
    pub cmd_set: AtomicU64,
    pub get_hits: AtomicU64,
    pub get_misses: AtomicU64,
    pub bytes_read: AtomicU64,
    pub bytes_written: AtomicU64,
}

impl AtomicCounters {
    pub fn new() -> Self {
        Self {
            cmd_get: AtomicU64::new(0),
            cmd_set: AtomicU64::new(0),
            get_hits: AtomicU64::new(0),
            get_misses: AtomicU64::new(0),
            bytes_read: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn inc_cmd_get(&self) {
        self.cmd_get.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_cmd_set(&self) {
        self.cmd_set.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_get_hits(&self) {
        self.get_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn inc_get_misses(&self) {
        self.get_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_bytes_read(&self, n: u64) {
        self.bytes_read.fetch_add(n, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_bytes_written(&self, n: u64) {
        self.bytes_written.fetch_add(n, Ordering::Relaxed);
    }
}

impl Default for AtomicCounters {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let metrics = Metrics::new();
        metrics.cmd_get.inc();
        metrics.cmd_set.inc();
        metrics.get_hits.inc();
        metrics.active_connections.set(5);

        let output = metrics.gather();
        assert!(output.contains("petracache_cmd_get_total"));
        assert!(output.contains("petracache_active_connections"));
    }

    #[test]
    fn test_atomic_counters() {
        let counters = AtomicCounters::new();
        counters.inc_cmd_get();
        counters.inc_cmd_get();
        counters.inc_get_hits();

        assert_eq!(counters.cmd_get.load(Ordering::Relaxed), 2);
        assert_eq!(counters.get_hits.load(Ordering::Relaxed), 1);
    }
}
