# Nimbus

**Blazing-fast, zero-copy RPC framework built in Rust, powered by RKYV serialization and ntex async networking.**

[![Crates.io](https://img.shields.io/crates/v/nimbus.svg)](https://crates.io/crates/nimbus)
[![Documentation](https://docs.rs/nimbus/badge.svg)](https://docs.rs/nimbus)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

## Features

- **Zero-copy serialization** via [rkyv](https://rkyv.org/) - O(1) message access with no parsing overhead
- **Ergonomic API** - Define services with `#[service]` macro on traits
- **Multiple transports** - TCP, UDP, Unix sockets with optional TLS
- **Full streaming support** - Unary, server-streaming, client-streaming, bidirectional
- **Production ready** - Service discovery, middleware/interceptors, connection pooling
- **Runtime flexibility** - Supports both tokio and compio (io_uring) runtimes

## Quick Start

Add Nimbus to your `Cargo.toml`:

```toml
[dependencies]
nimbus = "0.1"
rkyv = "0.8"
tokio = { version = "1", features = ["full"] }
```

Define and implement your service:

```rust
use nimbus::prelude::*;

#[derive(Archive, Serialize, Deserialize)]
pub struct CalcError(String);

#[service]
pub trait Calculator {
    async fn add(&self, a: i32, b: i32) -> Result<i32, CalcError>;
    async fn multiply(&self, a: i32, b: i32) -> Result<i32, CalcError>;
}

struct CalculatorImpl;

impl Calculator for CalculatorImpl {
    async fn add(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        Ok(a + b)
    }

    async fn multiply(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        Ok(a * b)
    }
}
```

Run the server:

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = TcpServer::new(CalculatorServer::new(CalculatorImpl));
    server.bind("0.0.0.0:9000").run().await?;
    Ok(())
}
```

Make client calls:

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = CalculatorClient::new(
        TcpClient::connect("127.0.0.1:9000".parse()?).await?
    );

    let ctx = Context::with_timeout(Duration::from_secs(10));
    let result = client.add(ctx, 2, 3).await?;
    println!("2 + 3 = {}", result);

    Ok(())
}
```

## Streaming RPCs

Nimbus supports all streaming patterns:

```rust
#[service]
pub trait DataService {
    // Server streaming - server sends multiple responses
    #[server_stream]
    async fn list_items(&self, filter: Filter) -> Result<Item, Error>;

    // Client streaming - client sends multiple requests
    #[client_stream]
    async fn upload(&self, #[stream] data: Chunk) -> Result<Summary, Error>;

    // Bidirectional streaming
    #[bidirectional]
    async fn chat(&self, #[stream] msg: Message) -> Result<Message, Error>;
}
```

## Middleware

Add cross-cutting concerns with interceptors:

```rust
use nimbus::{InterceptorChain, TimeoutInterceptor, RetryInterceptor, RetryConfig};

let interceptors = InterceptorChain::new()
    .with(TimeoutInterceptor::new(Duration::from_secs(30)))
    .with(RetryInterceptor::new(RetryConfig::default()));
```

## Service Discovery

Find services dynamically:

```rust
use nimbus::DnsResolver;

let resolver = DnsResolver::new();
let endpoints = resolver.resolve("_myservice._tcp.example.com").await?;
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `tokio` (default) | Use tokio runtime |
| `compio` | Use compio runtime for io_uring support |
| `tls` | Enable TLS via rustls |
| `discovery-dns` | DNS-based service discovery |
| `discovery-registry` | Registry-based service discovery |
| `tracing` | Distributed tracing support |
| `full` | Enable all features |

## Performance

Nimbus achieves exceptional performance through:

- **Zero-copy deserialization**: rkyv allows direct access to serialized data without copying or parsing
- **Minimal allocations**: Buffer pooling and careful memory management
- **Efficient framing**: Simple length-prefixed protocol
- **Modern async I/O**: Built on ntex with optional io_uring support

## Architecture

```
nimbus/
├── nimbus/              # Main crate (re-exports)
├── nimbus-core/         # Core types and traits
├── nimbus-codec/        # rkyv frame encoding
├── nimbus-transport/    # TCP, UDP, Unix transports
├── nimbus-macros/       # #[service] proc macro
├── nimbus-middleware/   # Interceptors
└── nimbus-discovery/    # Service discovery
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
