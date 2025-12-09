//! # nimbus-transport
//!
//! Transport implementations for the Nimbus RPC framework.
//!
//! This crate provides:
//! - [`TcpClient`] / [`TcpServer`] - TCP-based transport
//! - [`ConnectionPool`] - Client-side connection pooling (for Send types)
//! - [`Multiplexer`] - Request/response correlation
//! - Unix socket support (with `uds` feature)
//! - TLS support (with `tls` feature)
//!
//! ## Choosing a Transport
//!
//! | Transport | Use Case | Latency | Cross-Machine |
//! |-----------|----------|---------|---------------|
//! | TCP | General networking | Medium | Yes |
//! | TCP+TLS | Secure connections | Medium+ | Yes |
//! | UDS | Same-machine IPC | Low | No |
//!
//! **TCP** is the default choice for most deployments:
//! - Works across network boundaries
//! - Well-understood performance characteristics
//! - Compatible with load balancers and proxies
//!
//! **Unix Domain Sockets** (UDS) are optimal for local IPC:
//! - 30-50% lower latency than TCP loopback
//! - File-system based access control
//! - No network stack overhead
//!
//! **TLS** adds encryption with minimal overhead:
//! - Uses Rustls (pure Rust, no OpenSSL)
//! - Supports mutual TLS for client authentication
//! - Mozilla root certificates included
//!
//! ## Architecture Note
//!
//! The ntex framework uses single-threaded workers with `Rc`-based IO types.
//! This means `TcpConnection` and `UnixConnection` are not `Send` and should
//! be used within a single worker context. For multi-threaded access, create
//! connections per-worker.
//!
//! The [`ConnectionPool`] requires `Send` types and is designed for use cases
//! where connections can be safely shared across threads.
//!
//! ## Features
//!
//! - `tokio` (default) - Use tokio runtime
//! - `compio` - Use compio runtime for io_uring support
//! - `tls` - Enable TLS support via Rustls
//! - `uds` - Enable Unix domain socket support

mod mux;
mod pool;
mod tcp;

#[cfg(feature = "uds")]
mod uds;

#[cfg(feature = "tls")]
mod tls;

pub use mux::Multiplexer;
pub use pool::{ConnectionPool, PoolConfig, PoolStats, PooledConnection};
pub use tcp::{
    RequestHandler, TcpClient, TcpClientConfig, TcpConnection, TcpServer, TcpServerConfig,
};

#[cfg(feature = "uds")]
pub use uds::{UnixClient, UnixClientConfig, UnixConnection, UnixServer, UnixServerConfig};

#[cfg(feature = "tls")]
pub use tls::{ClientAuth, TlsClientConfig, TlsServerConfig, load_root_certs, server_name};
