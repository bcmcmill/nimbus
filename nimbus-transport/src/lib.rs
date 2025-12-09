//! # nimbus-transport
//!
//! Transport implementations for the Nimbus RPC framework.
//!
//! This crate provides:
//! - `TcpClient` / `TcpServer` - TCP-based transport
//! - `ConnectionPool` - Client-side connection pooling (for Send types)
//! - `Multiplexer` - Request/response correlation
//! - Unix socket support (with `uds` feature)
//! - TLS support (with `tls` feature)
//!
//! ## Architecture Note
//!
//! The ntex framework uses single-threaded workers with `Rc`-based IO types.
//! This means `TcpConnection` is not `Send` and should be used within a single
//! worker context. For multi-threaded access, create connections per-worker.
//!
//! ## Features
//!
//! - `tokio` (default) - Use tokio runtime
//! - `compio` - Use compio runtime for io_uring support
//! - `tls` - Enable TLS support via rustls
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
