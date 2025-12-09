//! # Nimbus
//!
//! Blazing-fast, zero-copy RPC framework built in Rust.
//!
//! Nimbus provides:
//! - **Zero-copy serialization** via [rkyv](https://rkyv.org/) for O(1) message access
//! - **Ergonomic API** with `#[service]` proc macro for defining RPC services
//! - **Multiple transports**: TCP, UDP, Unix sockets with optional TLS
//! - **Full streaming support**: unary, server-streaming, client-streaming, bidirectional
//! - **Production features**: service discovery, middleware/interceptors, connection pooling
//!
//! ## Quick Start
//!
//! ```rust
//! use nimbus::{Context, NimbusError, StaticResolver, Endpoint};
//! use std::net::SocketAddr;
//!
//! // Create a resolver for service discovery
//! let resolver = StaticResolver::new();
//! let addr: SocketAddr = "127.0.0.1:9000".parse().unwrap();
//! resolver.add_endpoint("my-service", addr);
//!
//! // Create a context for RPC calls
//! let ctx = Context::new();
//! assert!(ctx.deadline.is_none()); // No deadline by default
//!
//! // Context with timeout
//! let ctx_with_timeout = Context::with_timeout(std::time::Duration::from_secs(30));
//! assert!(ctx_with_timeout.deadline.is_some());
//! ```
//!
//! ## Features
//!
//! - `tokio` (default) - Use tokio runtime
//! - `compio` - Use compio runtime for io_uring support
//! - `tls` - Enable TLS support via rustls
//! - `discovery-dns` - Enable DNS-based service discovery
//! - `discovery-registry` - Enable registry-based service discovery
//! - `tracing` - Enable distributed tracing support
//! - `full` - Enable all features
//!
//! ## Architecture
//!
//! Nimbus is composed of several crates:
//!
//! - [`nimbus-core`] - Core types, traits, and error definitions
//! - [`nimbus-codec`] - rkyv-based frame encoding/decoding
//! - [`nimbus-transport`] - Transport implementations (TCP, UDP, Unix)
//! - [`nimbus-macros`] - Proc macros (`#[service]`)
//! - [`nimbus-middleware`] - Interceptors and middleware
//! - [`nimbus-discovery`] - Service discovery

// Re-export core types
pub use nimbus_core::{
    CodecError, Context, Metadata, NimbusError, StreamHandle, TraceId, Transport, TransportError,
};

// Re-export message types
pub use nimbus_core::{MessageType, RpcEnvelope};

// Re-export codec
pub use nimbus_codec::{AlignedBufferPool, AlignedVec, NimbusCodec, PooledBuffer};

// Re-export transport
pub use nimbus_transport::{
    ConnectionPool, Multiplexer, PoolConfig, PooledConnection, RequestHandler, TcpClient,
    TcpClientConfig, TcpConnection, TcpServer, TcpServerConfig,
};

// Re-export macros
pub use nimbus_macros::service;

// Re-export middleware
pub use nimbus_middleware::{
    Interceptor, InterceptorChain, InterceptorError, RetryConfig, RetryInterceptor,
    TimeoutInterceptor,
};

// Re-export discovery
pub use nimbus_discovery::{Endpoint, ResolveError, Resolver, StaticResolver};

#[cfg(feature = "discovery-dns")]
pub use nimbus_discovery::DnsResolver;

#[cfg(feature = "tracing")]
pub use nimbus_middleware::TracingInterceptor;

// Re-export rkyv for user convenience
pub use rkyv::{Archive, Deserialize, Serialize};

/// Prelude module for convenient imports.
///
/// ```rust
/// use nimbus::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        Archive, Context, Deserialize, NimbusError, Serialize, TcpClient, TcpServer, service,
    };

    pub use nimbus_core::Transport;
}

/// Version information.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
