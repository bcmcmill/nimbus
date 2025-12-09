//! Unix domain socket transport implementation.
//!
//! This module provides Unix domain socket support for local IPC,
//! offering lower latency than TCP for same-machine communication.
//!
//! ## When to Use UDS
//!
//! Unix domain sockets are ideal when:
//! - Client and server run on the same machine
//! - You need lower latency than TCP (no network stack overhead)
//! - You want file-system based access control
//! - You don't need to cross machine boundaries
//!
//! Use TCP instead when:
//! - Clients connect from remote machines
//! - You need network-level load balancing
//! - Cross-platform compatibility is required (Windows has limited UDS support)
//!
//! ## Performance
//!
//! UDS typically provides:
//! - ~30-50% lower latency than TCP loopback
//! - Higher throughput for small messages
//! - No TCP connection establishment overhead
//!
//! ## Client Example
//!
//! ```rust,ignore
//! use nimbus_transport::{UnixClient, UnixClientConfig};
//!
//! let client = UnixClient::new();
//! let conn = client.connect("/tmp/nimbus.sock").await?;
//!
//! // Use connection for RPC calls
//! conn.send(request_bytes).await?;
//! let response = conn.recv().await?;
//! ```
//!
//! ## Server Example
//!
//! ```rust,ignore
//! use nimbus_transport::{UnixServer, UnixServerConfig, RequestHandler};
//!
//! struct MyHandler;
//! impl RequestHandler for MyHandler {
//!     async fn handle(&self, request: AlignedVec) -> Result<Vec<u8>, NimbusError> {
//!         // Process request and return response
//!         Ok(vec![])
//!     }
//! }
//!
//! let config = UnixServerConfig::new("/tmp/my-service.sock")
//!     .workers(4)
//!     .max_connections(1000);
//!
//! let server = UnixServer::with_config(config, MyHandler);
//! server.run().await?;
//! ```
//!
//! ## Socket File Cleanup
//!
//! The server automatically removes any existing socket file before binding.
//! Ensure your application has appropriate permissions to create and remove
//! the socket file at the configured path.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::future::{Either, select};
use futures_timer::Delay;
use ntex::io::IoBoxed;
use ntex::service::fn_service;
use ntex::util::Buf;
use ntex_bytes::BytesMut;
use rkyv::util::AlignedVec;

use nimbus_codec::NimbusCodec;
use nimbus_core::{Context, NimbusError, TransportError};

use crate::mux::Multiplexer;
use crate::tcp::RequestHandler;

/// Configuration for Unix socket client.
#[derive(Debug, Clone)]
pub struct UnixClientConfig {
    /// Connection timeout.
    pub connect_timeout: Duration,

    /// Request timeout (if not specified in context).
    pub request_timeout: Duration,

    /// Maximum frame size.
    pub max_frame_size: usize,
}

impl Default for UnixClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            max_frame_size: 16 * 1024 * 1024,
        }
    }
}

/// Unix socket client for RPC communication.
///
/// Provides lower latency than TCP for local IPC by bypassing
/// the network stack.
///
/// ## Example
///
/// ```rust,ignore
/// use nimbus_transport::UnixClient;
///
/// let client = UnixClient::new();
/// let conn = client.connect("/tmp/nimbus.sock").await?;
/// ```
pub struct UnixClient {
    config: UnixClientConfig,
}

impl UnixClient {
    /// Create a new Unix socket client with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(UnixClientConfig::default())
    }

    /// Create a Unix socket client with custom configuration.
    #[must_use]
    pub fn with_config(config: UnixClientConfig) -> Self {
        Self { config }
    }

    /// Connect to a Unix socket and return a dedicated connection.
    pub async fn connect(&self, path: impl AsRef<Path>) -> Result<UnixConnection, TransportError> {
        let path = path.as_ref();

        // Connect with timeout using ntex's native unix_connect
        let connect_fut = pin!(ntex::rt::unix_connect(path));
        let timeout_fut = pin!(Delay::new(self.config.connect_timeout));

        let io = match select(connect_fut, timeout_fut).await {
            Either::Left((Ok(io), _)) => io,
            Either::Left((Err(e), _)) => {
                return Err(TransportError::ConnectionFailed(format!(
                    "failed to connect to {}: {}",
                    path.display(),
                    e
                )));
            }
            Either::Right(_) => {
                return Err(TransportError::ConnectionFailed(format!(
                    "connection to {} timed out",
                    path.display()
                )));
            }
        };

        Ok(UnixConnection::new(io.into(), self.config.max_frame_size))
    }
}

impl Default for UnixClient {
    fn default() -> Self {
        Self::new()
    }
}

/// A single Unix socket connection.
///
/// This type is designed for single-threaded use within ntex's worker model.
/// Use `Rc<UnixConnection>` if you need to share within a single worker.
pub struct UnixConnection {
    io: Rc<RefCell<IoBoxed>>,
    codec: NimbusCodec,
    mux: Rc<Multiplexer>,
}

impl UnixConnection {
    /// Create a new Unix connection from an IO handle.
    pub fn new(io: IoBoxed, max_frame_size: usize) -> Self {
        Self {
            io: Rc::new(RefCell::new(io)),
            codec: NimbusCodec::with_max_frame_size(max_frame_size),
            mux: Rc::new(Multiplexer::new()),
        }
    }

    /// Check if the connection is still open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.io.borrow().is_closed()
    }

    /// Close the connection.
    pub fn close(&self) {
        self.mux.cancel_all();
        self.io.borrow().close();
    }

    /// Send a raw frame over the connection.
    pub async fn send(&self, data: &[u8]) -> Result<(), TransportError> {
        let io = self.io.borrow();

        // Encode the frame
        let mut buf = BytesMut::with_capacity(4 + data.len());
        self.codec
            .encode_slice(data, &mut buf)
            .map_err(|e| TransportError::Io(Arc::new(std::io::Error::other(format!("{}", e)))))?;

        // Write to buffer and flush
        let _ = io
            .with_write_buf(|write_buf| {
                write_buf.extend_from_slice(&buf);
                Ok::<_, std::io::Error>(())
            })
            .map_err(|e| TransportError::Io(Arc::new(e)))?;

        io.flush(true)
            .await
            .map_err(|e| TransportError::Io(Arc::new(std::io::Error::other(format!("{}", e)))))?;

        Ok(())
    }

    /// Receive a raw frame from the connection.
    pub async fn recv(&self) -> Result<AlignedVec, TransportError> {
        let io = self.io.borrow();
        let codec = &self.codec;

        loop {
            // Try to decode from existing buffer (zero-copy path)
            let result: Result<Option<AlignedVec>, std::io::Error> =
                io.with_read_buf(|read_buf| match codec.decode_slice(read_buf.as_ref()) {
                    Ok(Some((frame, consumed))) => {
                        read_buf.advance(consumed);
                        Ok(Some(frame))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(std::io::Error::other(format!("{}", e))),
                });

            match result {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {
                    // Need more data, wait for read
                    io.read_ready().await.map_err(|e| {
                        TransportError::Io(Arc::new(std::io::Error::other(format!("{}", e))))
                    })?;

                    if io.is_closed() {
                        return Err(TransportError::ConnectionClosed);
                    }
                }
                Err(e) => return Err(TransportError::Io(Arc::new(e))),
            }
        }
    }

    /// Send a request and wait for a response (unary RPC pattern).
    pub async fn call(&self, ctx: &Context, request: &[u8]) -> Result<AlignedVec, NimbusError> {
        // Register pending request
        let (request_id, rx) = self.mux.register();

        // Build the request frame with request ID
        // In a full implementation, we'd wrap this in an RpcEnvelope
        let _ = request_id; // Used in envelope

        // Send request
        self.send(request).await.map_err(NimbusError::Transport)?;

        // Wait for response with timeout
        let timeout_duration = ctx.remaining().unwrap_or(Duration::from_secs(30));

        let rx_fut = pin!(rx);
        let timeout_fut = pin!(Delay::new(timeout_duration));

        match select(rx_fut, timeout_fut).await {
            Either::Left((Ok(result), _)) => result,
            Either::Left((Err(_), _)) => Err(NimbusError::Cancelled),
            Either::Right(_) => {
                self.mux.cancel(request_id);
                Err(NimbusError::Timeout(timeout_duration))
            }
        }
    }
}

/// Configuration for Unix socket server.
#[derive(Debug, Clone)]
pub struct UnixServerConfig {
    /// Path to the Unix socket file.
    pub socket_path: PathBuf,

    /// Number of worker threads.
    pub workers: usize,

    /// Maximum concurrent connections.
    pub max_connections: usize,

    /// Connection backlog.
    pub backlog: usize,

    /// Maximum frame size.
    pub max_frame_size: usize,
}

impl Default for UnixServerConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/tmp/nimbus.sock"),
            workers: num_cpus::get(),
            max_connections: 10000,
            backlog: 2048,
            max_frame_size: 16 * 1024 * 1024,
        }
    }
}

impl UnixServerConfig {
    /// Create a new config with the given socket path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: path.into(),
            ..Default::default()
        }
    }

    /// Set the socket path.
    #[must_use]
    pub fn socket_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.socket_path = path.into();
        self
    }

    /// Set the number of worker threads.
    #[must_use]
    pub fn workers(mut self, workers: usize) -> Self {
        self.workers = workers;
        self
    }

    /// Set the maximum concurrent connections.
    #[must_use]
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the maximum frame size.
    #[must_use]
    pub fn max_frame_size(mut self, size: usize) -> Self {
        self.max_frame_size = size;
        self
    }
}

/// Unix socket server for accepting RPC connections.
///
/// ## Example
///
/// ```rust,ignore
/// use nimbus_transport::{UnixServer, UnixServerConfig, RequestHandler};
///
/// struct MyHandler;
/// impl RequestHandler for MyHandler {
///     async fn handle(&self, request: AlignedVec) -> Result<Vec<u8>, NimbusError> {
///         Ok(vec![])
///     }
/// }
///
/// let config = UnixServerConfig::new("/tmp/my-service.sock");
/// let server = UnixServer::with_config(config, MyHandler);
/// server.run().await?;
/// ```
pub struct UnixServer<H> {
    config: UnixServerConfig,
    handler: Arc<H>,
}

impl<H> UnixServer<H>
where
    H: RequestHandler + Send + Sync + 'static,
{
    /// Create a new Unix socket server with the given path and handler.
    pub fn new(path: impl Into<PathBuf>, handler: H) -> Self {
        Self {
            config: UnixServerConfig::new(path),
            handler: Arc::new(handler),
        }
    }

    /// Create a server with custom configuration.
    pub fn with_config(config: UnixServerConfig, handler: H) -> Self {
        Self {
            config,
            handler: Arc::new(handler),
        }
    }

    /// Run the server.
    ///
    /// This will remove any existing socket file at the configured path
    /// before binding.
    pub async fn run(self) -> std::io::Result<()> {
        let handler = self.handler.clone();
        let max_frame_size = self.config.max_frame_size;

        // Remove stale socket file if it exists
        if self.config.socket_path.exists() {
            std::fs::remove_file(&self.config.socket_path)?;
        }

        ntex::server::build()
            .workers(self.config.workers)
            .maxconn(self.config.max_connections)
            .backlog(self.config.backlog as i32)
            .bind_uds("nimbus-uds", &self.config.socket_path, move |_| {
                let handler = handler.clone();
                fn_service(move |io: ntex::io::Io<_>| {
                    let handler = handler.clone();
                    async move {
                        let local_handler = Rc::new(handler);
                        handle_connection(io.into(), local_handler, max_frame_size).await;
                        Ok::<_, std::io::Error>(())
                    }
                })
            })?
            .run()
            .await
    }
}

async fn handle_connection<H>(io: IoBoxed, handler: Rc<Arc<H>>, max_frame_size: usize)
where
    H: RequestHandler + Send + Sync + 'static,
{
    let codec = NimbusCodec::with_max_frame_size(max_frame_size);

    tracing::debug!("Unix connection established");

    loop {
        // Wait for data to be available
        if io.read_ready().await.is_err() || io.is_closed() {
            tracing::debug!("Unix connection closed");
            break;
        }

        // Try to decode frames from the read buffer (zero-copy path)
        let frame_result: Result<Option<AlignedVec>, std::io::Error> =
            io.with_read_buf(|read_buf| match codec.decode_slice(read_buf.as_ref()) {
                Ok(Some((frame, consumed))) => {
                    read_buf.advance(consumed);
                    Ok(Some(frame))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(std::io::Error::other(format!("{}", e))),
            });

        match frame_result {
            Ok(Some(frame)) => {
                // Handle the request
                match handler.handle(frame).await {
                    Ok(response) => {
                        // Encode the response
                        let mut write_buf = BytesMut::with_capacity(4 + response.len());
                        if codec.encode_slice(&response, &mut write_buf).is_ok() {
                            // Write to IO buffer
                            let write_result = io.with_write_buf(|buf| {
                                buf.extend_from_slice(&write_buf);
                                Ok::<_, std::io::Error>(())
                            });

                            if write_result.is_err() {
                                tracing::debug!("Failed to write response");
                                break;
                            }

                            // Flush the buffer
                            if io.flush(true).await.is_err() {
                                tracing::debug!("Failed to flush response");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Handler error: {}", e);
                    }
                }
            }
            Ok(None) => {
                // No complete frame yet, continue waiting
            }
            Err(e) => {
                tracing::error!("Decode error: {}", e);
                break;
            }
        }
    }
}

/// Helper function to check CPU count.
mod num_cpus {
    pub fn get() -> usize {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_client_creation() {
        let client = UnixClient::new();
        assert_eq!(client.config.connect_timeout, Duration::from_secs(10));
        assert_eq!(client.config.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_unix_server_config() {
        let config = UnixServerConfig::new("/tmp/test.sock")
            .workers(4)
            .max_connections(1000)
            .max_frame_size(1024 * 1024);

        assert_eq!(config.socket_path, PathBuf::from("/tmp/test.sock"));
        assert_eq!(config.workers, 4);
        assert_eq!(config.max_connections, 1000);
        assert_eq!(config.max_frame_size, 1024 * 1024);
    }
}
