//! RocksDB storage backend
//!
//! Simple key-value store with RocksDB.

use crate::StorageError;
use crate::config::StorageConfig;
use crate::storage::value::{StoredValue, current_timestamp};
use rust_rocksdb::{BlockBasedOptions, CompactionDecision, DB, DBCompactionStyle, LogLevel, Options, WriteOptions};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, trace};

/// Global counter for TTL compaction removals (accessible from compaction filter)
pub static TTL_COMPACTION_REMOVED: AtomicU64 = AtomicU64::new(0);

/// Global counter for expired keys removed (lazy expiration + background scan)
pub static EXPIRED_KEYS_REMOVED: AtomicU64 = AtomicU64::new(0);

/// Memory usage statistics
#[derive(Debug, Clone, Default)]
pub struct MemoryUsage {
    /// Block cache usage in bytes
    pub block_cache_usage: usize,
    /// Total memory usage in bytes
    pub total: usize,
}

/// RocksDB-backed storage
pub struct RocksStorage {
    db: Arc<DB>,
    write_opts: WriteOptions,
}

impl RocksStorage {
    /// Open or create a RocksDB database
    pub fn open(config: &StorageConfig) -> Result<Self, StorageError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_max_background_jobs(config.max_background_jobs);
        opts.set_write_buffer_size(config.write_buffer_size);
        opts.set_max_write_buffer_number(config.max_write_buffer_number);
        opts.set_target_file_size_base(config.target_file_size_base);
        opts.set_compaction_style(DBCompactionStyle::Level);

        // RocksDB LOG file settings
        opts.set_log_level(parse_log_level(&config.rocksdb_log_level));
        opts.set_max_log_file_size(config.rocksdb_max_log_file_size);
        opts.set_keep_log_file_num(config.rocksdb_keep_log_file_num);

        if config.enable_compression {
            opts.set_compression_type(rust_rocksdb::DBCompressionType::Lz4);
        } else {
            opts.set_compression_type(rust_rocksdb::DBCompressionType::None);
        }

        // Block cache with optimized settings
        let mut block_opts = BlockBasedOptions::default();
        let cache = rust_rocksdb::Cache::new_lru_cache(config.block_cache_size);
        block_opts.set_block_cache(&cache);
        block_opts.set_bloom_filter(10.0, false);
        // Cache index and filter blocks in block cache (reduces disk I/O)
        block_opts.set_cache_index_and_filter_blocks(true);
        // Pin L0 filter and index blocks (prevents eviction of hot data)
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
        // Larger block size reduces metadata overhead
        block_opts.set_block_size(16 * 1024);
        opts.set_block_based_table_factory(&block_opts);

        // TTL compaction filter
        if config.enable_ttl_compaction {
            opts.set_compaction_filter("ttl_filter", ttl_compaction_filter);
        }

        // Ensure the directory exists
        if let Some(parent) = config.db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::Internal(format!("Failed to create directory: {e}"))
            })?;
        }

        let db = DB::open(&opts, &config.db_path)?;

        info!(
            "RocksDB opened: path={:?}, block_cache={}MB",
            config.db_path,
            config.block_cache_size / (1024 * 1024),
        );

        // Disable WAL: writes go directly to memtable (RAM only)
        // Data reaches disk only when memtable flushes to SST file (~every few seconds)
        // Trade-off: crash loses unflushed data (acceptable for a cache)
        let mut write_opts = WriteOptions::default();
        write_opts.disable_wal(true);

        Ok(Self { db: Arc::new(db), write_opts })
    }

    /// Get a value by key (with lazy expiration)
    pub fn get(&self, key: &[u8]) -> Result<Option<StoredValue>, StorageError> {
        match self.db.get(key)? {
            Some(bytes) => {
                let value = StoredValue::decode(&bytes)?;
                if value.is_expired() {
                    EXPIRED_KEYS_REMOVED.fetch_add(1, Ordering::Relaxed);
                    info!(
                        key = %String::from_utf8_lossy(key),
                        expire_at = value.expire_at,
                        "Lazy expiration: removed expired key"
                    );
                    let _ = self.db.delete_opt(key, &self.write_opts);
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
            None => Ok(None),
        }
    }

    /// Get multiple values by keys using batched MultiGet API
    pub fn get_multi(
        &self,
        keys: &[Vec<u8>],
    ) -> Result<Vec<(Vec<u8>, Option<StoredValue>)>, StorageError> {
        // Use RocksDB's native multi_get for better performance
        // (batches lookups, reduces mutex contention, enables parallel I/O)
        let raw_results = self.db.multi_get(keys);

        let mut results = Vec::with_capacity(keys.len());
        let mut expired_keys = Vec::new();

        for (key, raw_result) in keys.iter().zip(raw_results.into_iter()) {
            match raw_result {
                Ok(Some(bytes)) => {
                    let value = StoredValue::decode(&bytes)?;
                    if value.is_expired() {
                        expired_keys.push(key.clone());
                        results.push((key.clone(), None));
                    } else {
                        results.push((key.clone(), Some(value)));
                    }
                }
                Ok(None) => {
                    results.push((key.clone(), None));
                }
                Err(e) => {
                    return Err(StorageError::RocksDb(e));
                }
            }
        }

        // Batch delete expired keys (lazy expiration)
        if !expired_keys.is_empty() {
            EXPIRED_KEYS_REMOVED.fetch_add(expired_keys.len() as u64, Ordering::Relaxed);
            for key in &expired_keys {
                trace!(
                    key = %String::from_utf8_lossy(key),
                    "Lazy expiration: removed expired key"
                );
                let _ = self.db.delete_opt(key, &self.write_opts);
            }
        }

        Ok(results)
    }

    /// Set a value (WAL disabled — writes go to memtable only, flushed to disk async)
    pub fn set(&self, key: &[u8], value: StoredValue) -> Result<(), StorageError> {
        let encoded = value.encode();
        self.db.put_opt(key, &encoded, &self.write_opts)?;
        Ok(())
    }

    /// Delete a key
    ///
    /// Returns `true` if the key existed, `false` otherwise.
    /// Note: This is not fully atomic - between get and delete another thread
    /// could modify the key. For memcached semantics this is acceptable.
    pub fn delete(&self, key: &[u8]) -> Result<bool, StorageError> {
        let existed = self.db.get(key)?.is_some();
        // Always call delete - RocksDB delete is idempotent
        // This avoids the race where key is deleted between get and delete
        self.db.delete_opt(key, &self.write_opts)?;
        Ok(existed)
    }

    /// Get memory usage statistics
    pub fn memory_usage(&self) -> MemoryUsage {
        let block_cache_usage = self
            .db
            .property_int_value("rocksdb.block-cache-usage")
            .unwrap_or(None)
            .unwrap_or(0) as usize;

        MemoryUsage {
            block_cache_usage,
            total: block_cache_usage,
        }
    }

    /// Get TTL expiration statistics
    pub fn ttl_stats() -> TtlStats {
        TtlStats {
            expired_removed: EXPIRED_KEYS_REMOVED.load(Ordering::Relaxed),
            compaction_removed: TTL_COMPACTION_REMOVED.load(Ordering::Relaxed),
        }
    }

    /// Manually trigger compaction (useful for testing TTL compaction)
    pub fn compact(&self) {
        info!("Starting manual compaction");
        self.db.compact_range::<&[u8], &[u8]>(None, None);
        info!(
            compaction_removed = TTL_COMPACTION_REMOVED.load(Ordering::Relaxed),
            "Manual compaction completed"
        );
    }
}

/// TTL expiration statistics
#[derive(Debug, Clone, Default)]
pub struct TtlStats {
    /// Keys removed by lazy expiration or background scan
    pub expired_removed: u64,
    /// Keys removed by compaction filter
    pub compaction_removed: u64,
}

fn parse_log_level(level: &str) -> LogLevel {
    match level.to_lowercase().as_str() {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "fatal" => LogLevel::Fatal,
        "header" => LogLevel::Header,
        _ => LogLevel::Error, // "error" or any unknown value
    }
}

/// TTL compaction filter - removes expired entries during compaction
fn ttl_compaction_filter(_level: u32, _key: &[u8], value: &[u8]) -> CompactionDecision {
    if value.len() >= 8 {
        let expire_at = u64::from_le_bytes(value[0..8].try_into().unwrap_or([0; 8]));

        if expire_at != 0 && current_timestamp() >= expire_at {
            TTL_COMPACTION_REMOVED.fetch_add(1, Ordering::Relaxed);
            return CompactionDecision::Remove;
        }
    }
    CompactionDecision::Keep
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp_dir: &TempDir) -> StorageConfig {
        StorageConfig {
            db_path: tmp_dir.path().join("test_db"),
            block_cache_size: 8 * 1024 * 1024,
            write_buffer_size: 4 * 1024 * 1024,
            max_write_buffer_number: 2,
            target_file_size_base: 4 * 1024 * 1024,
            max_background_jobs: 2,
            enable_compression: false,
            enable_ttl_compaction: false,
            rocksdb_log_level: "error".to_string(),
            rocksdb_max_log_file_size: 10 * 1024 * 1024,
            rocksdb_keep_log_file_num: 5,
        }
    }

    #[test]
    fn test_set_get() {
        let tmp_dir = TempDir::new().unwrap();
        let storage = RocksStorage::open(&test_config(&tmp_dir)).unwrap();

        let value = StoredValue::new(42, 0, b"hello".to_vec());
        storage.set(b"test_key", value).unwrap();

        let result = storage.get(b"test_key").unwrap();
        assert!(result.is_some());
        let v = result.unwrap();
        assert_eq!(v.flags, 42);
        assert_eq!(v.data, b"hello");
    }

    #[test]
    fn test_get_nonexistent() {
        let tmp_dir = TempDir::new().unwrap();
        let storage = RocksStorage::open(&test_config(&tmp_dir)).unwrap();

        let result = storage.get(b"nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete() {
        let tmp_dir = TempDir::new().unwrap();
        let storage = RocksStorage::open(&test_config(&tmp_dir)).unwrap();

        assert!(!storage.delete(b"nonexistent").unwrap());

        let value = StoredValue::new(0, 0, b"data".to_vec());
        storage.set(b"key", value).unwrap();

        assert!(storage.delete(b"key").unwrap());
        assert!(storage.get(b"key").unwrap().is_none());
    }

    #[test]
    fn test_compaction_filter_expired_key() {
        // expire_at = 1 (far in the past), flags = 0, data = "old"
        let value = StoredValue::with_expire_at(0, 1, b"old".to_vec());
        let encoded = value.encode();

        let decision = ttl_compaction_filter(0, b"key", &encoded);
        assert!(matches!(decision, CompactionDecision::Remove));
    }

    #[test]
    fn test_compaction_filter_valid_key() {
        // expire_at far in the future
        let value = StoredValue::with_expire_at(0, u64::MAX, b"fresh".to_vec());
        let encoded = value.encode();

        let decision = ttl_compaction_filter(0, b"key", &encoded);
        assert!(matches!(decision, CompactionDecision::Keep));
    }

    #[test]
    fn test_compaction_filter_never_expire() {
        // expire_at = 0 means never expire
        let value = StoredValue::with_expire_at(0, 0, b"permanent".to_vec());
        let encoded = value.encode();

        let decision = ttl_compaction_filter(0, b"key", &encoded);
        assert!(matches!(decision, CompactionDecision::Keep));
    }

    #[test]
    fn test_compaction_filter_short_value() {
        // Value too short to contain expire_at header
        let decision = ttl_compaction_filter(0, b"key", &[0, 1, 2]);
        assert!(matches!(decision, CompactionDecision::Keep));
    }
}
