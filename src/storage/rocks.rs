//! RocksDB storage backend
//!
//! Simple key-value store with RocksDB.

use crate::StorageError;
use crate::config::StorageConfig;
use crate::storage::value::{StoredValue, current_timestamp};
use rust_rocksdb::{BlockBasedOptions, CompactionDecision, DB, DBCompactionStyle, Options};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, trace};

/// Global counter for TTL compaction removals (accessible from compaction filter)
pub static TTL_COMPACTION_REMOVED: AtomicU64 = AtomicU64::new(0);

/// Global counter for lazy expiration removals
pub static LAZY_EXPIRATION_REMOVED: AtomicU64 = AtomicU64::new(0);

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

        if config.enable_compression {
            opts.set_compression_type(rust_rocksdb::DBCompressionType::Lz4);
        } else {
            opts.set_compression_type(rust_rocksdb::DBCompressionType::None);
        }

        // Block cache
        let mut block_opts = BlockBasedOptions::default();
        let cache = rust_rocksdb::Cache::new_lru_cache(config.block_cache_size);
        block_opts.set_block_cache(&cache);
        block_opts.set_bloom_filter(10.0, false);
        opts.set_block_based_table_factory(&block_opts);

        // TTL compaction filter
        if config.enable_ttl_compaction {
            opts.set_compaction_filter("ttl_filter", ttl_compaction_filter);
        }

        // Ensure the directory exists
        if let Some(parent) = config.db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::Internal(format!("Failed to create directory: {}", e))
            })?;
        }

        let db = DB::open(&opts, &config.db_path)?;

        info!(
            "RocksDB opened: path={:?}, block_cache={}MB",
            config.db_path,
            config.block_cache_size / (1024 * 1024),
        );

        Ok(Self { db: Arc::new(db) })
    }

    /// Get a value by key (with lazy expiration)
    pub fn get(&self, key: &[u8]) -> Result<Option<StoredValue>, StorageError> {
        match self.db.get(key)? {
            Some(bytes) => {
                let value = StoredValue::decode(&bytes)?;
                if value.is_expired() {
                    LAZY_EXPIRATION_REMOVED.fetch_add(1, Ordering::Relaxed);
                    info!(
                        key = %String::from_utf8_lossy(key),
                        expire_at = value.expire_at,
                        "Lazy expiration: removed expired key"
                    );
                    let _ = self.db.delete(key);
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
            None => Ok(None),
        }
    }

    /// Get multiple values by keys
    pub fn get_multi(
        &self,
        keys: &[Vec<u8>],
    ) -> Result<Vec<(Vec<u8>, Option<StoredValue>)>, StorageError> {
        let mut results = Vec::with_capacity(keys.len());
        let mut expired_keys = Vec::new();

        for key in keys {
            match self.db.get(key)? {
                Some(bytes) => {
                    let value = StoredValue::decode(&bytes)?;
                    if value.is_expired() {
                        expired_keys.push(key.clone());
                        results.push((key.clone(), None));
                    } else {
                        results.push((key.clone(), Some(value)));
                    }
                }
                None => {
                    results.push((key.clone(), None));
                }
            }
        }

        for key in &expired_keys {
            LAZY_EXPIRATION_REMOVED.fetch_add(1, Ordering::Relaxed);
            info!(
                key = %String::from_utf8_lossy(key),
                "Lazy expiration: removed expired key"
            );
            let _ = self.db.delete(key);
        }

        Ok(results)
    }

    /// Set a value
    pub fn set(&self, key: &[u8], value: StoredValue) -> Result<(), StorageError> {
        let encoded = value.encode();
        self.db.put(key, &encoded)?;
        Ok(())
    }

    /// Delete a key
    pub fn delete(&self, key: &[u8]) -> Result<bool, StorageError> {
        let exists = if let Some(bytes) = self.db.get(key)? {
            if let Ok(existing) = StoredValue::decode(&bytes) {
                !existing.is_expired()
            } else {
                true
            }
        } else {
            false
        };

        if exists {
            self.db.delete(key)?;
        }
        Ok(exists)
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
            lazy_expiration_removed: LAZY_EXPIRATION_REMOVED.load(Ordering::Relaxed),
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
    /// Keys removed by lazy expiration (on GET)
    pub lazy_expiration_removed: u64,
    /// Keys removed by compaction filter
    pub compaction_removed: u64,
}

/// TTL compaction filter - removes expired entries during compaction
fn ttl_compaction_filter(_level: u32, key: &[u8], value: &[u8]) -> CompactionDecision {
    if value.len() >= 8 {
        let expire_at = u64::from_le_bytes(value[0..8].try_into().unwrap_or([0; 8]));

        if expire_at != 0 && current_timestamp() >= expire_at {
            TTL_COMPACTION_REMOVED.fetch_add(1, Ordering::Relaxed);
            trace!(
                key = %String::from_utf8_lossy(key),
                expire_at = expire_at,
                "TTL compaction: removing expired key"
            );
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
}
