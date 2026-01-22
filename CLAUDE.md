# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

RocksProxy is a high-performance Rust server that speaks memcached ASCII protocol with RocksDB as the storage backend. Designed to work behind mcrouter for routing and failover.

```
┌──────────────┐     ┌───────────┐     ┌─────────────────────────┐
│ app/service  │────▶│ mcrouter  │────▶│ RocksProxy (this)       │
│ (memcache    │     │ (routing, │     │  ├─ ASCII protocol      │
│  client)     │     │  failover)│     │  ├─ TTL support         │
└──────────────┘     └───────────┘     │  └─ RocksDB backend     │
                                       └─────────────────────────┘
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

- `get <key>*` / `gets <key>*` - Multi-key support
- `set`, `add`, `replace` - `<key> <flags> <exptime> <bytes> [noreply]`
- `delete <key> [noreply]`
- `incr` / `decr` - `<key> <value> [noreply]`
- `touch <key> <exptime> [noreply]`
- `flush_all [delay] [noreply]`
- `version`, `stats`, `quit`

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
├── lib.rs            # Library root, error types (RocksProxyError, StorageError, ProtocolError)
├── config.rs         # Configuration (ServerConfig, StorageConfig, MetricsConfig)
├── server.rs         # TCP server, connection handling, command execution
├── protocol/
│   ├── mod.rs
│   ├── parser.rs     # Hand-written ASCII parser (zero-copy with Cow)
│   ├── command.rs    # Command enum with is_noreply(), into_owned()
│   └── response.rs   # ResponseWriter for building memcached responses
├── storage/
│   ├── mod.rs
│   ├── rocks.rs      # RocksDB backend with spawn_blocking, TTL compaction filter
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
