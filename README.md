# PetraCache

High-performance memcached-compatible cache server backed by RocksDB, written in Rust.

> *Petra* (πέτρα) means "rock" in Greek - a nod to the RocksDB storage engine.

PetraCache speaks the memcached ASCII protocol and uses RocksDB as its storage backend, making it ideal for use behind mcrouter for routing and failover.

```
┌──────────────┐     ┌───────────┐     ┌─────────────────────────┐
│ app/service  │────▶│ mcrouter  │────▶│ PetraCache              │
│ (memcache    │     │ (routing, │     │  ├─ ASCII protocol      │
│  client)     │     │  failover)│     │  ├─ TTL support         │
└──────────────┘     └───────────┘     │  └─ RocksDB backend     │
                                       └─────────────────────────┘
```

## Features

- **Memcached ASCII Protocol**: Compatible with standard memcached clients
- **RocksDB Backend**: Persistent, high-performance key-value storage
- **TTL Support**: Automatic expiration with lazy deletion and compaction filter
- **mcrouter Compatible**: Works with mcrouter for routing and failover
- **Prometheus Metrics**: Built-in metrics endpoint
- **Health Checks**: HTTP endpoints for liveness and readiness probes
- **Zero-Copy Parsing**: Efficient protocol parsing with minimal allocations

## Requirements

- Rust 1.92+ (Edition 2024)
- C++ compiler (for RocksDB compilation)

## Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/petracache.git
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

| Command | Format | Description |
|---------|--------|-------------|
| `get` | `get <key>*` | Retrieve one or more keys |
| `gets` | `gets <key>*` | Retrieve with CAS token |
| `set` | `set <key> <flags> <exptime> <bytes> [noreply]` | Store a key |
| `add` | `add <key> <flags> <exptime> <bytes> [noreply]` | Store if not exists |
| `replace` | `replace <key> <flags> <exptime> <bytes> [noreply]` | Store if exists |
| `delete` | `delete <key> [noreply]` | Delete a key |
| `incr` | `incr <key> <value> [noreply]` | Increment numeric value |
| `decr` | `decr <key> <value> [noreply]` | Decrement numeric value |
| `touch` | `touch <key> <exptime> [noreply]` | Update expiration |
| `flush_all` | `flush_all [delay] [noreply]` | Invalidate all keys |
| `version` | `version` | Server version |
| `stats` | `stats` | Server statistics |
| `quit` | `quit` | Close connection |

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

## Performance Targets

| Metric | Target |
|--------|--------|
| Throughput | >100K ops/sec |
| Latency p50 | <0.5ms |
| Latency p99 | <2ms |
| Latency p99.9 | <5ms |
| Connections | 10K+ |

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
├── lib.rs            # Library root, error types
├── config.rs         # Configuration handling
├── server/
│   ├── mod.rs        # TCP server
│   ├── connection.rs # Connection handling
│   └── handler.rs    # Command handlers
├── protocol/
│   ├── mod.rs
│   ├── parser.rs     # ASCII protocol parser
│   ├── command.rs    # Command definitions
│   └── response.rs   # Response formatting
├── storage/
│   ├── mod.rs
│   ├── rocks.rs      # RocksDB backend
│   └── value.rs      # Value encoding/decoding
├── metrics.rs        # Prometheus metrics
└── health.rs         # Health check server
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
