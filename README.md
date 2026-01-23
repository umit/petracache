# PetraCache

**Memcached speed. Persistent storage. Zero complexity.**

High-performance in-memory cache with persistent storage, designed to run behind mcrouter. Handle millions of requests per second with data durability.

> *Petra* (πέτρα) means "rock" in Greek - a nod to the RocksDB storage engine.

## Comparison

| Feature | Memcached | Redis | PetraCache |
|---------|-----------|-------|------------|
| Protocol | Memcached | Redis | Memcached |
| Persistence | ❌ | ✅ | ✅ |
| In-Memory Speed | ✅ | ✅ | ✅ |
| mcrouter Support | ✅ | ❌ | ✅ |
| Drop-in Replacement | - | ❌ | ✅ |
| Complexity | Low | High | Low |

### Key Differentiators

- **Drop-in replacement** for memcached with persistence
- **Built for mcrouter** - Facebook's battle-tested cache router
- **Written in Rust** - Memory safe, zero GC pauses, predictable latency

## Why PetraCache?

**The Problem:** Memcached is super fast but doesn't persist data. Redis offers persistence but with a different protocol and added complexity.

**The Solution:** PetraCache combines the best of both worlds:
- **In-Memory Performance**: RocksDB's block cache keeps hot data in memory
- **Persistent Storage**: Data survives restarts, no cold cache problem
- **Memcached Protocol**: Drop-in replacement, works with existing clients
- **mcrouter Ready**: Built for distributed caching with routing and failover

```
┌──────────────┐     ┌───────────┐     ┌─────────────────────────┐
│ app/service  │────▶│ mcrouter  │────▶│ PetraCache              │
│ (memcache    │     │ (routing, │     │  ├─ In-memory (fast)    │
│  client)     │     │  failover)│     │  ├─ Persistent (safe)   │
└──────────────┘     └───────────┘     │  └─ TTL support         │
                                       └─────────────────────────┘
```

## Use Cases

- **High-Traffic Web Applications**: Handle millions of requests with sub-millisecond latency
- **Real-Time Data Pipelines**: Fast key-value lookups for streaming data processing
- **Microservices Architecture**: Shared state and caching layer across services
- **Gaming & Ad Tech**: Low-latency data access for real-time bidding and game state
- **Financial Systems**: High-throughput transaction caching with durability
- **IoT Data Ingestion**: Buffer and store high-volume sensor data

## Features

- **In-Memory + Persistent**: Hot data in memory via RocksDB block cache, all data persisted to disk
- **Memcached Protocol**: Drop-in replacement for memcached, works with any memcached client
- **mcrouter Integration**: Designed for distributed caching with Facebook's mcrouter
- **TTL Support**: Memcached-compatible expiration (lazy deletion + compaction filter)
- **High Performance**: Zero-copy parsing, efficient buffer management, async I/O
- **Production Ready**: Prometheus metrics, health checks, graceful shutdown
- **Configurable Memory**: Tune block cache size based on your memory budget

## Requirements

- Rust 1.85+ (Edition 2024)
- C++ compiler (for RocksDB compilation)

## Installation

```bash
# Clone the repository
git clone https://github.com/umit/petracache.git
cd petracache

# Build release version
cargo build --release

# Run
./target/release/petracache config.toml
```

## Usage

### Running the Server

```bash
# With configuration file
./petracache config.toml

# With environment variables
PETRACACHE_LISTEN_ADDR=127.0.0.1:11211 \
PETRACACHE_DB_PATH=./data/rocksdb \
./petracache
```

### Connecting with a Client

```bash
# Using netcat
echo -e "set foo 0 0 3\r\nbar\r\n" | nc localhost 11211
echo -e "get foo\r\n" | nc localhost 11211

# Using memcached client libraries (any language)
```

## Supported Commands

### Implemented

| Command | Format | Description |
|---------|--------|-------------|
| `get` | `get <key>*` | Retrieve one or more keys |
| `set` | `set <key> <flags> <exptime> <bytes> [noreply]` | Store a key |
| `delete` | `delete <key> [noreply]` | Delete a key |
| `version` | `version` | Server version (used by mcrouter health checks) |
| `quit` | `quit` | Close connection |

### Planned

| Command | Format | Description |
|---------|--------|-------------|
| `add` | `add <key> <flags> <exptime> <bytes> [noreply]` | Store only if key doesn't exist |
| `replace` | `replace <key> <flags> <exptime> <bytes> [noreply]` | Store only if key exists |
| `append` | `append <key> <flags> <exptime> <bytes> [noreply]` | Append data to existing key |
| `prepend` | `prepend <key> <flags> <exptime> <bytes> [noreply]` | Prepend data to existing key |
| `incr` | `incr <key> <value> [noreply]` | Increment numeric value |
| `decr` | `decr <key> <value> [noreply]` | Decrement numeric value |
| `touch` | `touch <key> <exptime> [noreply]` | Update expiration time |
| `gets` | `gets <key>*` | Retrieve with CAS token |
| `cas` | `cas <key> <flags> <exptime> <bytes> <cas> [noreply]` | Compare and swap |
| `stats` | `stats` | Server statistics |
| `flush_all` | `flush_all [delay] [noreply]` | Invalidate all keys |

## Configuration

Create a `config.toml` file:

```toml
[server]
listen_addr = "127.0.0.1:11211"
max_connections = 10000
read_buffer_size = 8192
write_buffer_size = 8192

[storage]
db_path = "./data/rocksdb"
block_cache_size = 1073741824  # 1GB
write_buffer_size = 67108864   # 64MB
max_write_buffer_number = 3
target_file_size_base = 67108864  # 64MB
max_background_jobs = 4
enable_compression = false
enable_ttl_compaction = true

[metrics]
enabled = true
listen_addr = "127.0.0.1:9090"
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PETRACACHE_LISTEN_ADDR` | Server listen address | `127.0.0.1:11211` |
| `PETRACACHE_MAX_CONNECTIONS` | Max concurrent connections | `10000` |
| `PETRACACHE_DB_PATH` | RocksDB data directory | `./data/rocksdb` |
| `PETRACACHE_METRICS_ADDR` | Metrics server address | `127.0.0.1:9090` |
| `PETRACACHE_METRICS_ENABLED` | Enable metrics server | `true` |

## TTL Expiration

PetraCache supports memcached-compatible TTL expiration:

- **exptime = 0**: Never expire
- **exptime <= 2592000** (30 days): Relative seconds from now
- **exptime > 2592000**: Absolute Unix timestamp

Expired keys are removed via:
1. **Lazy expiration**: Keys are deleted when accessed after expiration
2. **Compaction filter**: RocksDB removes expired keys during compaction

## HTTP Endpoints

When metrics are enabled, the following endpoints are available:

| Endpoint | Description |
|----------|-------------|
| `/health` | Liveness probe (always returns 200) |
| `/ready` | Readiness probe |
| `/metrics` | Prometheus metrics |

## Performance

PetraCache is designed for high-throughput scenarios. When deployed behind mcrouter with multiple instances, you can scale horizontally to handle **millions of requests per second**.

### Single Instance Targets

| Metric | Target |
|--------|--------|
| Throughput | >100K ops/sec |
| Latency p50 | <0.5ms |
| Latency p99 | <2ms |
| Latency p99.9 | <5ms |
| Connections | 10K+ |

### Horizontal Scaling with mcrouter

```
                         ┌─────────────────┐
                    ┌───▶│ PetraCache #1   │
                    │    └─────────────────┘
┌─────────────┐     │    ┌─────────────────┐
│  mcrouter   │─────┼───▶│ PetraCache #2   │  = Millions of TPS
└─────────────┘     │    └─────────────────┘
                    │    ┌─────────────────┐
                    └───▶│ PetraCache #N   │
                         └─────────────────┘
```

Scale by adding more PetraCache instances behind mcrouter with consistent hashing.

## Benchmarking

```bash
memtier_benchmark \
  -s 127.0.0.1 -p 11211 \
  --protocol=memcache_text \
  --clients=50 --threads=4 \
  --test-time=60 --ratio=1:1 --data-size=100
```

## Project Structure

```
src/
├── main.rs           # Entry point
├── lib.rs            # Library root
├── error.rs          # Error types (PetraCacheError, ProtocolError, StorageError)
├── prelude.rs        # Common imports
├── config.rs         # Configuration handling
├── server/
│   ├── mod.rs        # TCP server, accept loop
│   ├── connection.rs # Connection handling, read/write loops
│   └── handler.rs    # Command handlers
├── protocol/
│   ├── mod.rs
│   ├── parser.rs     # Hand-written ASCII protocol parser
│   ├── command.rs    # Command definitions
│   └── response.rs   # Response formatting
├── storage/
│   ├── mod.rs
│   ├── rocks.rs      # RocksDB backend, TTL compaction filter
│   └── value.rs      # Value encoding/decoding
├── metrics.rs        # Prometheus metrics
└── health.rs         # HTTP health server (/health, /ready, /metrics)
```

## Building from Source

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run with logging
RUST_LOG=info cargo run -- config.toml

# Run with trace logging (includes TTL compaction)
RUST_LOG=trace cargo run -- config.toml
```

## License

MIT License - see [LICENSE](LICENSE) for details.
