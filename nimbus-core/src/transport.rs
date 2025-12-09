//! Transport trait definitions.
//!
//! The `Transport` trait abstracts over different network transports
//! (TCP, UDP, Unix sockets, etc.) for RPC communication.

use std::future::Future;
use std::pin::Pin;

use rkyv::util::AlignedVec;

use crate::context::Context;
use crate::error::NimbusError;

/// Handle for bidirectional streaming.
pub struct StreamHandle {
    /// Channel for sending stream items.
    pub sender: Box<dyn StreamSender>,
    /// Channel for receiving stream items.
    pub receiver: Box<dyn StreamReceiver>,
}

/// Trait for sending stream items.
pub trait StreamSender: Send + Sync {
    /// Send a message on the stream.
    fn send(
        &self,
        data: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<(), NimbusError>> + Send + '_>>;

    /// Close the send side of the stream.
    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), NimbusError>> + Send + '_>>;
}

/// Trait for receiving stream items.
pub trait StreamReceiver: Send + Sync {
    /// Receive the next message from the stream.
    /// Returns `None` when the stream is closed.
    fn recv(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<AlignedVec>, NimbusError>> + Send + '_>>;
}

/// Core transport trait for RPC communication.
///
/// Implementations handle the actual network communication,
/// including connection management, framing, and serialization.
///
/// # Example
///
/// ```rust
/// use nimbus_core::{Transport, Context};
/// use rkyv::util::AlignedVec;
///
/// // The Transport trait defines the interface for RPC communication
/// fn example_signature<T: Transport>(_transport: &T) {
///     // Transport implementations provide:
///     // - call() for unary RPC
///     // - open_stream() for streaming RPC
///     // - is_connected() for health checks
///     // - close() for cleanup
/// }
/// ```
pub trait Transport: Send + Sync {
    /// Error type for transport operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Send a request and wait for a response (unary RPC).
    ///
    /// # Arguments
    /// * `ctx` - Request context with deadline, tracing, metadata
    /// * `request` - Serialized request bytes
    ///
    /// # Returns
    /// An aligned buffer containing the response, suitable for zero-copy rkyv access.
    fn call(
        &self,
        ctx: &Context,
        request: &[u8],
    ) -> impl Future<Output = Result<AlignedVec, Self::Error>> + Send;

    /// Open a bidirectional stream for streaming RPC.
    ///
    /// # Arguments
    /// * `ctx` - Request context
    ///
    /// # Returns
    /// A `StreamHandle` for sending and receiving stream messages.
    fn open_stream(
        &self,
        ctx: &Context,
    ) -> impl Future<Output = Result<StreamHandle, Self::Error>> + Send;

    /// Check if the transport is connected and healthy.
    fn is_connected(&self) -> bool;

    /// Close the transport connection.
    fn close(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Server-side transport for accepting connections.
pub trait ServerTransport: Send + Sync {
    /// Error type for server operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Connection type returned when accepting.
    type Connection: Transport;

    /// Accept a new connection.
    fn accept(&self) -> impl Future<Output = Result<Self::Connection, Self::Error>> + Send;

    /// Get the local address the server is bound to.
    fn local_addr(&self) -> Result<std::net::SocketAddr, Self::Error>;

    /// Shutdown the server.
    fn shutdown(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Builder for creating transports with common configuration.
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// Maximum frame size in bytes.
    pub max_frame_size: usize,

    /// Connection timeout.
    pub connect_timeout: std::time::Duration,

    /// Read timeout for individual operations.
    pub read_timeout: Option<std::time::Duration>,

    /// Write timeout for individual operations.
    pub write_timeout: Option<std::time::Duration>,

    /// Enable TCP nodelay.
    pub nodelay: bool,

    /// TCP keepalive interval.
    pub keepalive: Option<std::time::Duration>,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            max_frame_size: 16 * 1024 * 1024, // 16 MB
            connect_timeout: std::time::Duration::from_secs(10),
            read_timeout: None,
            write_timeout: None,
            nodelay: true,
            keepalive: Some(std::time::Duration::from_secs(60)),
        }
    }
}

impl TransportConfig {
    /// Create a new transport configuration with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum frame size.
    #[must_use]
    pub fn max_frame_size(mut self, size: usize) -> Self {
        self.max_frame_size = size;
        self
    }

    /// Set the connection timeout.
    #[must_use]
    pub fn connect_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the read timeout.
    #[must_use]
    pub fn read_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.read_timeout = Some(timeout);
        self
    }

    /// Set the write timeout.
    #[must_use]
    pub fn write_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.write_timeout = Some(timeout);
        self
    }

    /// Enable or disable TCP nodelay.
    #[must_use]
    pub fn nodelay(mut self, enabled: bool) -> Self {
        self.nodelay = enabled;
        self
    }

    /// Set the TCP keepalive interval.
    #[must_use]
    pub fn keepalive(mut self, interval: std::time::Duration) -> Self {
        self.keepalive = Some(interval);
        self
    }
}
