# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

PetraCache is a high-performance Rust server that speaks memcached ASCII protocol with RocksDB as the storage backend. Designed to work behind mcrouter for routing and failover.

> *Petra* (πέτρα) means "rock" in Greek - a nod to the RocksDB storage engine.

```
┌──────────────┐     ┌───────────┐     ┌─────────────────────────┐
│ app/service  │────▶│ mcrouter  │────▶│ PetraCache (this)       │
│ (memcache    │     │ (routing, │     │  ├─ ASCII protocol      │
│  client)     │     │  failover)│     │  ├─ TTL support         │
└──────────────┘     └───────────┘     │  └─ RocksDB backend     │
                                       └─────────────────────────┘
```

## What is mcrouter?

[mcrouter](https://github.com/facebook/mcrouter) is Facebook's open-source memcached protocol router. At Facebook scale:

- **5 billion requests/second** across thousands of memcached servers
- Powers Facebook, Instagram, WhatsApp caching infrastructure
- Battle-tested at extreme scale since 2013

**mcrouter capabilities (out of the box):**

- **Autoscaling**: Add/remove nodes with minimal key redistribution (consistent hashing)
- **Multi-zone/Multi-region**: Route to nearest zone, cross-region replication
- **Replication**: Synchronous or async writes to N replicas
- **Failover**: Automatic detection (TKO), instant routing to healthy replicas
- **Traffic splitting**: A/B testing, shadow traffic, gradual rollouts
- **Connection pooling**: Thousands of app connections → fewer backend connections
- **Request collapsing**: Deduplicate identical concurrent requests

**Why PetraCache sits behind mcrouter:**

| Feature | mcrouter handles | PetraCache handles |
|---------|------------------|-------------------|
| Sharding | Consistent hashing across nodes | Single node storage |
| Replication | Replicate writes to N servers | Store data reliably |
| Failover | Detect failures, route to replicas | Respond to health checks |
| Connection pooling | Multiplex client connections | Handle fewer connections |
| Request routing | Prefix routing, shadow traffic | Execute commands |
| Multi-zone | Route to nearest datacenter | Be fast in one zone |

**mcrouter does the distributed systems work**, PetraCache focuses on being a fast, reliable single-node cache with persistence.

```
Client request flow:
1. App sends "get user:123" to mcrouter
2. mcrouter hashes "user:123" → determines target server
3. mcrouter sends request to PetraCache node
4. PetraCache reads from RocksDB, returns value
5. mcrouter returns response to app
```

## Build Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo run                      # Run application
cargo test                     # Run all tests
cargo test test_name           # Run specific test
cargo check                    # Check without building
cargo fmt                      # Format code
cargo clippy                   # Run linter
cargo bench                    # Run benchmarks
```

## Technical Requirements

- **Rust Edition**: 2024 (Rust 1.92+)
- **Async Runtime**: Tokio
- **Error Handling**: thiserror + anyhow
- **ASCII Protocol Parser**: Hand-written (no external library)

## Supported Memcached Commands

- `get <key>*` - Retrieve values (multi-key support)
- `set <key> <flags> <exptime> <bytes> [noreply]\r\n<data>\r\n` - Store value
- `delete <key> [noreply]` - Delete key
- `version` - Returns server version (used by mcrouter for health checks)
- `quit` - Close connection

## mcrouter Integration

PetraCache is designed to work behind [mcrouter](https://github.com/facebook/mcrouter), Facebook's memcached protocol router.

**Health Check Behavior:**
- mcrouter uses `version` command to probe server health
- When a server times out consecutively, mcrouter marks it "TKO" (technical knockout)
- mcrouter sends periodic `version` probes to detect recovery
- Once `version` responds, mcrouter restores the server to active pool

**Example mcrouter config:**
```json
{
  "pools": {
    "A": {
      "servers": ["127.0.0.1:11211"]
    }
  },
  "route": "PoolRoute|A"
}
```

## Storage Format

RocksDB value format: `[8 bytes: expire_at][4 bytes: flags][N bytes: data]`

**TTL Rules (memcached-compatible):**
- 0 = never expire
- <= 2592000 (30 days) = relative seconds
- > 2592000 = absolute Unix timestamp

**Expiration Strategy:**
- Lazy expiration on GET (check & delete if expired)
- Compaction filter for background cleanup

## Project Structure

```
src/
├── main.rs           # Entry point, server initialization
├── lib.rs            # Library root, error types (PetraCacheError, StorageError, ProtocolError)
├── config.rs         # Configuration (ServerConfig, StorageConfig, MetricsConfig)
├── server/
│   ├── mod.rs        # TCP server, accept loop
│   ├── connection.rs # Connection handling, read/write loops
│   └── handler.rs    # Command handlers (handle_get, handle_set, etc.)
├── protocol/
│   ├── mod.rs
│   ├── parser.rs     # Hand-written ASCII parser (zero-copy with Cow)
│   ├── command.rs    # Command enum with is_noreply(), into_owned()
│   └── response.rs   # ResponseWriter for building memcached responses
├── storage/
│   ├── mod.rs
│   ├── rocks.rs      # RocksDB backend, TTL compaction filter
│   └── value.rs      # StoredValue encoding/decoding, TTL calculation
├── metrics.rs        # Prometheus metrics + AtomicCounters for hot paths
└── health.rs         # HTTP health server (/health, /ready, /metrics)
```

## macOS Build Notes

The `.cargo/config.toml` automatically sets the required C++ compiler flags for RocksDB compilation on macOS. No manual environment variables needed.

## Critical Implementation Patterns

### Zero-Copy Parsing
```rust
// Use Cow for keys to avoid allocations
use std::borrow::Cow;

pub enum Command<'a> {
    Get { keys: Vec<Cow<'a, [u8]>> },
    Set { key: Cow<'a, [u8]>, ... },
}
```

### Buffer Management
```rust
// Reuse BytesMut per connection, don't allocate on each request
struct Connection {
    read_buf: BytesMut,
    write_buf: BytesMut,
}
```

### Async I/O - Handle Partial Reads
Always loop until `\r\n` is found - single read may return incomplete data.

### RocksDB in Async Context
```rust
// RocksDB calls are blocking - use spawn_blocking
async fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
    let db = Arc::clone(&self.db);
    let key = key.to_vec();
    tokio::task::spawn_blocking(move || {
        db.get(&key).ok().flatten()
    }).await.ok().flatten()
}
```

### Connection Limits
Use `tokio::sync::Semaphore` for max concurrent connections (target: 10K+).

### Metrics
Use `AtomicU64` with `Ordering::Relaxed` for counters - no Mutex.

### Struct Layout
Order fields from largest to smallest alignment to minimize padding:
```rust
struct StoredValue {
    expire_at: u64,  // 8 bytes first
    flags: u32,      // 4 bytes
    data: Vec<u8>,
}
```

## Benchmarking

```bash
# Basic SET/GET test
memtier_benchmark \
  -s 127.0.0.1 -p 11211 \
  --protocol=memcache_text \
  --clients=50 --threads=4 \
  --test-time=60 --ratio=1:1 --data-size=100
```

## Performance Targets

| Metric        | Target       |
|---------------|--------------|
| Throughput    | >100K ops/sec|
| Latency p50   | <0.5ms       |
| Latency p99   | <2ms         |
| Latency p99.9 | <5ms         |
| Connections   | 10K+         |

## Current Status

### Implemented
- **GET**: Single and multi-key retrieval with batched RocksDB multi_get
- **SET**: With flags, TTL support (memcached-compatible semantics)
- **DELETE**: With noreply support
- **VERSION**: Returns server version (mcrouter health check)
- **QUIT**: Graceful connection close
- **TTL Expiration**: Lazy expiration on GET + compaction filter cleanup
- **Health Server**: HTTP endpoints at /health, /ready, /metrics
- **Prometheus Metrics**: ops counters, latency histograms, connection tracking
- **Graceful Shutdown**: SIGINT/SIGTERM handling with connection draining

### Not Implemented
- **STATS**: mcrouter uses this for monitoring (returns empty for now)
- **FLUSH_ALL**: Clear all keys
- **ADD/REPLACE**: Conditional storage commands
- **APPEND/PREPEND**: Data modification commands
- **INCR/DECR**: Atomic counters
- **CAS**: Check-and-set (optimistic locking)
- **Binary Protocol**: Only ASCII protocol supported
- **TOUCH**: Update TTL without fetching value
- **GAT/GATS**: Get-and-touch operations

## Known Issues

1. **Lazy expiration logging**: Uses `info!` level, should be `trace!` for production
2. **No buffer size limit**: Unbounded read buffer could be DoS vector
3. **DELETE race condition**: Mitigated but not fully atomic (acceptable for memcached semantics)
4. **No key size validation on storage**: Parser validates, but storage layer doesn't double-check

## TODO / Roadmap

### High Priority (mcrouter compatibility)
- [ ] Implement STATS command (basic stats mcrouter expects)
- [ ] Add FLUSH_ALL command

### Medium Priority (features)
- [ ] ADD/REPLACE commands
- [ ] INCR/DECR commands
- [ ] Buffer size limit configuration
- [ ] Change lazy expiration log level to trace!

### Low Priority (optimization)
- [ ] Use AtomicCounters instead of Prometheus mutex for hot-path metrics
- [ ] Connection pooling for health checks
- [ ] Configurable compaction filter behavior

## Development Principles

### No Premature Optimization
- **Measure first**: Don't optimize without benchmark data proving the bottleneck
- **Show evidence**: "X is slow" requires numbers (latency, throughput, CPU profile)
- **Avoid hype**: Don't claim "50% improvement" without before/after benchmarks
- **Simple first**: Working solution > clever solution. Optimize only when needed

### Code Quality
- **SOLID principles**: Single responsibility, open/closed, dependency injection
- **Clean code**: Meaningful names, small functions, no magic numbers
- **TDD mindset**: Write tests for new features, especially parser and storage
- **No over-engineering**: Don't add abstractions for hypothetical future needs

### Database Concepts (must understand)
- **WAL (Write-Ahead Log)**: Writes go to WAL first, then memtable
- **LSM Tree**: How RocksDB organizes data (memtable → L0 → L1 → ...)
- **Compaction**: Background merge of SST files, when TTL filter runs
- **Block Cache**: LRU cache for frequently accessed blocks
- **Bloom Filter**: Probabilistic structure to avoid disk reads for missing keys
- **MVCC**: Multi-version concurrency (RocksDB snapshots)
- **Write Amplification**: Ratio of actual disk writes to user writes

### Rust Knowledge (must understand)

**Ownership & Lifetimes**
- `Cow<'a, [u8]>`: Borrow when possible, clone when needed (zero-copy parsing)
- `Arc<T>`: Shared ownership across async tasks (server, storage, metrics)
- `'static` bounds: Required for spawned tasks

**Async/Tokio**
- `tokio::select!`: Cancel-safe operations, connection handling with shutdown
- `tokio::spawn`: Task spawning, when to use vs inline async
- `AsyncReadExt/AsyncWriteExt`: `read_buf`, `write_all` patterns
- Backpressure: Semaphore for connection limits, bounded channels

**TCP/Networking**
- `TcpListener::bind` + `accept` loop pattern
- `TcpStream`: Non-blocking reads may return partial data (always loop until `\r\n`)
- `SO_REUSEADDR`, `TCP_NODELAY` for low latency
- Graceful shutdown: CancellationToken pattern

**Concurrency**
- `AtomicU64` with `Ordering::Relaxed`: Fast counters without locks
- `Mutex` vs `RwLock`: Mutex simpler, RwLock for read-heavy (but has overhead)
- `parking_lot`: Faster mutex implementation (consider for hot paths)
- No `spawn_blocking` for fast operations (<100µs)

**Performance Patterns**
- `BytesMut`: Reusable buffer, `split_to` for zero-copy consumption
- `memchr`: SIMD-accelerated byte search
- Avoid allocations in hot path: reuse buffers, use references
- `#[inline]` for small, frequently-called functions
- Struct field ordering: largest alignment first (reduce padding)

**Error Handling**
- `thiserror`: Define error enums with `#[error]` derive
- `anyhow`: For application-level errors with context
- `?` operator: Propagate errors, don't unwrap in library code

**Testing**
- `#[cfg(test)]` modules in same file
- `tempfile::TempDir` for RocksDB tests
- `tokio::test` for async tests

### Memcached Protocol Knowledge
- **Text protocol format**: `<command> <key> [args]\r\n`
- **TTL semantics**: 0=never, ≤30days=relative, >30days=absolute timestamp
- **noreply**: Fire-and-forget mode, no response sent
- **Multi-get**: Single `get key1 key2 key3` more efficient than 3 separate gets

### Distributed Systems Knowledge

**Consistency & Availability**
- CAP theorem: Partition tolerance required, choose C or A
- PetraCache chooses AP: Available under partition, eventual consistency via mcrouter
- Replication: mcrouter handles (not PetraCache's job)

**Failure Modes**
- Network partition: mcrouter detects via VERSION timeout, marks TKO
- Node crash: mcrouter failover to replica, cold cache on restart (RocksDB helps)
- Slow node: Timeout-based detection, not Byzantine fault tolerant

**mcrouter Specifics**
- Consistent hashing: Key → server mapping, minimal redistribution on node add/remove
- TKO (Technical Knockout): Server marked down after consecutive failures
- Failover: Automatic routing to healthy replicas
- Shadow traffic: Test new servers without affecting production

**Cache Patterns**
- Cache-aside: App reads cache, on miss reads DB and populates cache
- Write-through: Write to cache and DB together
- Write-behind: Write to cache, async write to DB
- Thundering herd: Many requests hit DB on cache miss (use locking/coalescing)
- Cache warming: Pre-populate cache after restart

### High Performance Concepts

**CPU & Memory**
- Cache lines: 64 bytes, access adjacent data together
- L1/L2/L3 cache: L1 ~1ns, L2 ~3ns, L3 ~10ns, RAM ~100ns
- False sharing: Two threads modify same cache line (use padding)
- Branch prediction: Predictable branches are fast, random branches hurt
- Prefetching: Sequential access patterns prefetch automatically

**System Calls & I/O**
- Syscall overhead: ~100-200ns per call, batch when possible
- `epoll`/`kqueue`: O(1) event notification (Tokio uses this)
- `io_uring`: Linux async I/O (future optimization)
- `mmap` vs `read`: mmap for random access, read for sequential
- `TCP_NODELAY`: Disable Nagle's algorithm for low latency

**Memory Allocation**
- Default allocator: Good for most cases
- `jemalloc`/`mimalloc`: Better for multi-threaded, fragmentation-prone workloads
- Arena allocation: Pre-allocate, reset instead of free (not used here)
- Object pooling: Reuse buffers instead of allocating (BytesMut pattern)

**Profiling & Measurement**
```bash
# CPU profiling (Linux)
perf record -g ./target/release/petracache
perf report

# Flamegraph
cargo flamegraph --bin petracache

# Memory profiling
valgrind --tool=massif ./target/release/petracache
heaptrack ./target/release/petracache

# Syscall tracing
strace -c ./target/release/petracache
```

**Benchmarking Rules**
- Warm up: Discard first N seconds of results
- Steady state: Run long enough (30-60s minimum)
- Percentiles: p50, p99, p99.9 matter more than average
- Isolate variables: Change one thing at a time
- Same machine load: Don't run benchmark + server on same CPU (M1 lesson)

### LSM Tree Deep Dive (RocksDB)

**Write Path**
1. Write to WAL (fsync for durability)
2. Insert to MemTable (in-memory sorted structure)
3. When MemTable full → flush to L0 SST file
4. Background compaction merges L0 → L1 → L2...

**Read Path**
1. Check MemTable (most recent writes)
2. Check L0 SST files (may check all, not sorted)
3. Check L1+ with binary search (sorted, non-overlapping)
4. Bloom filter: Skip files that definitely don't have key

**Compaction Styles**
- Level: Default, good read performance, higher write amplification
- Universal: Better write amplification, more space amplification
- FIFO: TTL-based, deletes oldest files (good for time-series)

**Tuning Tradeoffs**
- More write buffers → better write throughput, more memory
- Larger block cache → better read performance, more memory
- Compression → less disk I/O, more CPU
- Bloom filter bits → fewer false positives, more memory

### Observability & Monitoring

**Key Metrics to Watch**
- `ops_total`: Throughput (ops/sec)
- `latency_p99`: Tail latency (should be <2ms)
- `cache_hit_ratio`: Hits / (hits + misses), target >95%
- `active_connections`: Current open connections
- `rocksdb.block-cache-usage`: Memory pressure indicator
- `compaction_pending_bytes`: Compaction backlog

**Alerting Thresholds**
- p99 latency > 5ms: Investigate immediately
- Hit ratio < 90%: Cache sizing or access pattern issue
- Block cache usage > 90%: Increase cache or add nodes
- Pending compaction > 1GB: Compaction can't keep up

**Debug Commands**
```bash
# Check if server responds
echo "version" | nc localhost 11211

# Test SET/GET manually
echo -e "set foo 0 0 3\r\nbar\r" | nc localhost 11211
echo -e "get foo\r" | nc localhost 11211

# Check RocksDB stats (via metrics endpoint)
curl http://localhost:9090/metrics | grep rocksdb
```

### Security Considerations

**No Authentication**
- Memcached protocol has no auth - network isolation required
- Run in private network / VPC only
- Use firewall rules to restrict access

**DoS Prevention**
- Connection limit via Semaphore (max_connections config)
- TODO: Add max key size validation (250 bytes)
- TODO: Add max value size limit (1MB default)
- TODO: Add read buffer size limit

**Data Safety**
- No encryption at rest (RocksDB stores plaintext)
- No encryption in transit (memcached protocol is plaintext)
- Don't store sensitive data (passwords, PII) without app-level encryption

### Operations

**Deployment**
```bash
# Build release binary
cargo build --release

# Run with config
./target/release/petracache config.toml

# Graceful shutdown (waits for connections to drain)
kill -SIGTERM <pid>
```

**RocksDB Backup**
```bash
# RocksDB checkpoint (consistent snapshot)
# TODO: Implement checkpoint command

# Filesystem backup (stop server first for consistency)
rsync -av ./data/rocksdb/ /backup/rocksdb/
```

**Disk Full Recovery**
1. Stop server
2. Free disk space or expand volume
3. Delete old WAL files if needed: `rm ./data/rocksdb/*.log`
4. Restart server (may trigger recovery)

**Version Upgrade**
1. Build new version
2. Stop old server (graceful shutdown)
3. Start new server (RocksDB handles format compatibility)

### Capacity Planning

**Memory Calculation**
```
Block cache: Primary memory consumer
  - Rule of thumb: 10-20% of dataset size for good hit ratio
  - 1GB cache can serve ~10GB dataset with 90%+ hit ratio (if access is skewed)

Per-connection: ~16KB (read buffer + write buffer)
  - 10K connections = 160MB

Total = block_cache_size + (connections × 16KB) + ~100MB overhead
```

**Disk Calculation**
```
Raw data size × write_amplification × (1 + space_amplification)
  - write_amplification: ~10-30x for level compaction
  - space_amplification: ~10-20% temporary during compaction

Example: 10GB data → need ~15GB disk minimum
```

**Throughput Estimation**
```
Single node (8 cores, SSD):
  - Read-heavy (90% GET): 100-200K ops/sec
  - Write-heavy (90% SET): 50-100K ops/sec
  - Mixed (50/50): 70-150K ops/sec

Scaling: Linear with nodes (via mcrouter sharding)
  - 10 nodes → 1-2M ops/sec
```

### Common Pitfalls

**OOM (Out of Memory)**
- Cause: Block cache too large, or memory leak
- Diagnose: Check `rocksdb.block-cache-usage` metric
- Fix: Reduce `block_cache_size` in config

**Slow Compaction**
- Cause: Write rate exceeds compaction rate
- Symptoms: Growing L0 files, increased read latency
- Diagnose: Check `compaction_pending_bytes`
- Fix: Increase `max_background_jobs`, use faster SSD

**WAL Growth**
- Cause: WAL not being garbage collected
- Symptoms: Disk usage grows even with deletes
- Fix: Usually automatic after flush; manual compaction if needed

**High Tail Latency**
- Cause: Compaction I/O, GC pause (not in Rust), lock contention
- Diagnose: Check if correlated with compaction, use `perf`
- Fix: Rate-limit compaction, use dedicated compaction threads

**Connection Storm**
- Cause: Many clients reconnect simultaneously (after network blip)
- Symptoms: Accept queue full, new connections rejected
- Fix: Client-side exponential backoff, increase `max_connections`

### Protocol Edge Cases

**Key Constraints**
- Max length: 250 bytes
- Allowed chars: Printable ASCII except space and control chars
- No whitespace, no newlines

**Value Constraints**
- Max size: 1MB (memcached default, we should enforce)
- Binary safe: Can contain any bytes including \0
- Flags: 32-bit unsigned integer (client-defined meaning)

**TTL Edge Cases**
- 0: Never expire
- 1-2592000 (30 days in seconds): Relative to now
- >2592000: Absolute Unix timestamp
- Negative: Treated as "already expired" (immediate delete)
- Overflow: u64 max = never expire effectively

**Multi-get Behavior**
- Returns only keys that exist
- Order may not match request order
- Missing keys are silently omitted (no error)

### Troubleshooting Guide

| Symptom | Possible Cause | Diagnosis | Fix |
|---------|---------------|-----------|-----|
| High p99 latency | Compaction | Check pending bytes | More background jobs |
| High p99 latency | Disk I/O | iostat, check SSD | Faster storage |
| Low hit ratio | Working set > cache | Check cache usage | Increase cache |
| Low hit ratio | Cold cache | Recent restart? | Wait for warmup |
| Memory growth | Block cache | Check metrics | Reduce cache size |
| Memory growth | Connection leak | Check active_connections | Fix client |
| Disk full | WAL accumulation | Check *.log files | Manual compaction |
| Connection refused | Max connections | Check semaphore | Increase limit |

### Testing Guidelines
```bash
# Unit tests for parser
cargo test parse_

# Integration tests for storage
cargo test rocks_

# Full server test with memtier
memtier_benchmark -s 127.0.0.1 -p 11211 --protocol=memcache_text \
  --clients=10 --threads=2 --test-time=10 --ratio=1:9
```



### Why RocksDB instead of in-memory?
- Persistence across restarts (no cold cache problem)
- Memory-efficient for large datasets (block cache + disk)
- Built-in compression (LZ4)
- Production-proven at scale (Facebook, Netflix, etc.)

### Why ASCII protocol only?
- mcrouter primarily uses ASCII protocol
- Simpler to debug with telnet/nc
- Binary protocol adds complexity with minimal benefit for our use case

### Why hand-written parser instead of nom/pest?
- Zero-copy parsing with Cow<[u8]>
- No external dependencies for protocol layer
- Full control over error messages
- memcached protocol is simple enough

### Why no spawn_blocking for RocksDB?
- RocksDB reads from block cache are typically <100µs
- spawn_blocking overhead (~5-10µs) adds latency
- Tested: direct calls perform better for cache-hot workloads
- May reconsider for disk-heavy workloads with large datasets

### TTL storage format
```
[8 bytes: expire_at][4 bytes: flags][N bytes: data]
```
- expire_at first: compaction filter can skip decoding data
- Fixed-size header: O(1) access to metadata
