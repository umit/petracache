# RocksDB Tuning Guide for PetraCache

## Memory Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    RocksDB Memory Layout                     │
├─────────────────────────────────────────────────────────────┤
│  Block Cache (65%)         → READ performance               │
│  Write Buffers (15%)       → WRITE performance              │
│  Index/Filters (10%)       → Key lookup speed               │
│  App/Tokio (10%)           → Connections, runtime           │
└─────────────────────────────────────────────────────────────┘
                         ↓ Flush
┌─────────────────────────────────────────────────────────────┐
│                      SST Files (Disk)                        │
│  Level 0 → Level 1 → Level 2 → ... (Compaction)             │
└─────────────────────────────────────────────────────────────┘
```

## Memory Budget Formula

```
Total Available = TOTAL_RAM - OS_RESERVED (2GB min)

Block Cache     = Available × 0.65  (65%)  → Read performance
Write Buffers   = Available × 0.15  (15%)  → Write performance
Index/Filters   = Available × 0.10  (10%)  → Lookups
App/Tokio       = Available × 0.10  (10%)  → Connections

Write Buffer Count = Write Buffers Total / 256MB (min 2, max 8)
```

### Memory Budget Table

| Total RAM | Available | Block Cache | Write Buffers | Index/Filter | App/Tokio |
|-----------|-----------|-------------|---------------|--------------|-----------|
| **8GB**   | 6GB       | 4GB         | 1GB (4×256MB) | 600MB        | 600MB     |
| **16GB**  | 14GB      | 9GB         | 2GB (8×256MB) | 1.4GB        | 1.4GB     |
| **32GB**  | 30GB      | 19GB        | 4.5GB (8×512MB)| 3GB         | 3GB       |
| **64GB**  | 62GB      | 40GB        | 9GB (8×1GB)   | 6GB          | 6GB       |
| **128GB** | 126GB     | 82GB        | 19GB (8×2GB)  | 12GB         | 12GB      |
| **256GB** | 254GB     | 165GB       | 38GB (8×4GB)  | 25GB         | 25GB      |

## Recommended Configuration Changes

### 1. Write Durability

**Current:** No sync, possible data loss on crash

**Recommended:**
```rust
pub struct StorageConfig {
    pub sync_writes: bool,           // fsync on each write
    pub wal_dir: Option<PathBuf>,    // Separate WAL disk (SSD recommended)
    pub wal_sync_mode: WalSyncMode,  // None/Normal/Full
}

pub enum WalSyncMode {
    None,    // No sync (fastest, data loss possible)
    Normal,  // Sync on buffer full (balanced)
    Full,    // Sync every write (slowest, fully durable)
}
```

### 2. Write Buffer Tuning

**Current:**
```rust
write_buffer_size: 64MB
max_write_buffer_number: 3
// Total: 192MB write buffer
```

**Recommended for high-write workload:**
```rust
write_buffer_size: 128MB             // Larger buffer
max_write_buffer_number: 4           // More buffers
min_write_buffer_number_to_merge: 2  // NEW: Merge efficiency
```

### 3. Block Cache - Pin Hot Data

**Current:** Basic LRU cache only

**Recommended:**
```rust
let mut block_opts = BlockBasedOptions::default();
let cache = Cache::new_lru_cache(config.block_cache_size);
block_opts.set_block_cache(&cache);
block_opts.set_bloom_filter(10.0, false);

// NEW: Pin L0 data in cache
block_opts.set_cache_index_and_filter_blocks(true);
block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
// L0 blocks always stay in memory - fastest access
```

### 4. Level-based Compression

**Current:** All or nothing compression

**Recommended:**
```rust
// L0-L1: No compression (hot data, fast access)
// L2+: LZ4 compression (cold data, space saving)
opts.set_compression_per_level(&[
    DBCompressionType::None,  // L0 - active writes
    DBCompressionType::None,  // L1 - hot reads
    DBCompressionType::Lz4,   // L2 - warm data
    DBCompressionType::Lz4,   // L3 - cold data
    DBCompressionType::Lz4,   // L4 - coldest
    DBCompressionType::Zstd,  // L5+ - archive (optional)
]);
```

### 5. Rate Limiter - Prevent Compaction Storm

**Current:** No rate limiting, compaction can saturate disk

**Recommended:**
```rust
use rust_rocksdb::RateLimiter;

let rate_limiter = RateLimiter::new(
    100 * 1024 * 1024,  // 100MB/s compaction limit
    10_000,              // refill period (microseconds)
    10                   // fairness
);
opts.set_rate_limiter(&rate_limiter);
```

### 6. Direct I/O

**Current:** Using OS page cache (double caching)

**Recommended:**
```rust
// Bypass OS page cache - more predictable latency
// RocksDB block cache manages its own memory
opts.set_use_direct_reads(true);
opts.set_use_direct_io_for_flush_and_compaction(true);
```

**Note:** Requires block_cache to be properly sized, as OS won't cache anymore.

### 7. Statistics & Monitoring

**Current:** Basic memory usage only

**Recommended:**
```rust
opts.enable_statistics();
opts.set_stats_dump_period_sec(60);

// Expose these metrics:
// - rocksdb.block.cache.hit
// - rocksdb.block.cache.miss
// - rocksdb.block.cache.hit.ratio
// - rocksdb.compaction.pending
// - rocksdb.compaction.running
// - rocksdb.mem.table.size
// - rocksdb.num.live.sst.files
// - rocksdb.estimate.pending.compaction.bytes
```

## Proposed StorageConfig

```rust
pub struct StorageConfig {
    // Paths
    pub db_path: PathBuf,
    pub wal_dir: Option<PathBuf>,         // Separate WAL disk

    // Memory - Read Performance
    pub block_cache_size: usize,           // LRU cache for reads
    pub pin_l0_in_cache: bool,             // Keep L0 in memory
    pub cache_index_and_filters: bool,     // Cache metadata

    // Memory - Write Performance
    pub write_buffer_size: usize,          // Single memtable size
    pub max_write_buffer_number: i32,      // Total memtables
    pub min_write_buffer_to_merge: i32,    // Merge threshold

    // Durability
    pub sync_writes: bool,                 // fsync on write
    pub wal_sync_mode: WalSyncMode,        // WAL sync strategy

    // I/O Performance
    pub use_direct_io: bool,               // Bypass OS cache
    pub rate_limit_mb: Option<u64>,        // Compaction rate limit
    pub max_background_jobs: i32,          // Compaction threads

    // Compression
    pub compression_type: CompressionType,
    pub bottommost_compression: CompressionType,
    pub compression_per_level: bool,       // Level-based compression

    // TTL
    pub enable_ttl_compaction: bool,

    // Monitoring
    pub enable_statistics: bool,
    pub stats_dump_period_sec: u32,
}
```

## Config Examples

### 8GB Server (Small)
```toml
[storage]
db_path = "./data/rocksdb"
block_cache_size = 4294967296      # 4GB
write_buffer_size = 268435456       # 256MB
max_write_buffer_number = 4
use_direct_io = false               # Let OS help with small cache
compression_per_level = true
enable_statistics = true
```

### 64GB Server (Medium)
```toml
[storage]
db_path = "./data/rocksdb"
wal_dir = "/ssd/petracache/wal"     # Separate SSD for WAL
block_cache_size = 42949672960      # 40GB
write_buffer_size = 1073741824      # 1GB
max_write_buffer_number = 8
pin_l0_in_cache = true
cache_index_and_filters = true
use_direct_io = true
rate_limit_mb = 200                 # 200MB/s compaction
compression_per_level = true
enable_statistics = true
```

### 256GB Server (Large)
```toml
[storage]
db_path = "/nvme/petracache/data"
wal_dir = "/nvme2/petracache/wal"   # Separate NVMe for WAL
block_cache_size = 171798691840     # 160GB
write_buffer_size = 4294967296      # 4GB
max_write_buffer_number = 8
min_write_buffer_to_merge = 2
pin_l0_in_cache = true
cache_index_and_filters = true
use_direct_io = true
rate_limit_mb = 500                 # 500MB/s compaction
max_background_jobs = 8
compression_per_level = true
bottommost_compression = "zstd"
enable_statistics = true
stats_dump_period_sec = 30
```

## Implementation Priority

1. **High Priority**
   - [ ] Memory budget calculator (`MemoryBudget::from_gb()`)
   - [ ] Pin L0 in block cache
   - [ ] Cache index and filter blocks
   - [ ] RocksDB statistics export

2. **Medium Priority**
   - [ ] Level-based compression
   - [ ] Rate limiter for compaction
   - [ ] Separate WAL directory support
   - [ ] Direct I/O option

3. **Low Priority**
   - [ ] Auto-tuning based on available RAM
   - [ ] WAL sync modes
   - [ ] Custom compaction triggers

## Monitoring Metrics to Add

```prometheus
# Block Cache
petracache_rocksdb_block_cache_usage_bytes
petracache_rocksdb_block_cache_hit_total
petracache_rocksdb_block_cache_miss_total
petracache_rocksdb_block_cache_hit_ratio

# Write Path
petracache_rocksdb_memtable_size_bytes
petracache_rocksdb_memtable_count
petracache_rocksdb_wal_size_bytes

# Compaction
petracache_rocksdb_compaction_pending_bytes
petracache_rocksdb_compaction_running
petracache_rocksdb_num_sst_files

# Latency
petracache_rocksdb_get_micros_p50
petracache_rocksdb_get_micros_p99
petracache_rocksdb_write_micros_p50
petracache_rocksdb_write_micros_p99
```

## References

- [RocksDB Tuning Guide](https://github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide)
- [RocksDB Memory Usage](https://github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB)
- [RocksDB Write Buffer Manager](https://github.com/facebook/rocksdb/wiki/Write-Buffer-Manager)
- [RocksDB Block Cache](https://github.com/facebook/rocksdb/wiki/Block-Cache)
- [RocksDB Rate Limiter](https://github.com/facebook/rocksdb/wiki/Rate-Limiter)
