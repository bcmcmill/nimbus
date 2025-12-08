//! TCP transport implementation.

use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ntex::io::{Io, IoBoxed};
use ntex::service::fn_service;
use ntex::util::Buf;
use ntex::codec::Decoder;
use ntex::connect::{Connect, Connector};
use ntex_bytes::BytesMut;
use rkyv::util::AlignedVec;

use nimbus_codec::NimbusCodec;
use nimbus_core::{Context, NimbusError, TransportError};

use crate::mux::Multiplexer;

/// Configuration for TCP client.
#[derive(Debug, Clone)]
pub struct TcpClientConfig {
    /// Connection timeout.
    pub connect_timeout: Duration,

    /// Request timeout (if not specified in context).
    pub request_timeout: Duration,

    /// Enable TCP nodelay.
    pub nodelay: bool,

    /// Maximum frame size.
    pub max_frame_size: usize,
}

impl Default for TcpClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            nodelay: true,
            max_frame_size: 16 * 1024 * 1024,
        }
    }
}

/// TCP client for RPC communication.
///
/// Note: This client is designed to work within ntex's single-threaded
/// per-worker model. For multi-threaded access, create separate clients
/// per worker.
pub struct TcpClient {
    config: TcpClientConfig,
}

impl TcpClient {
    /// Create a new TCP client with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(TcpClientConfig::default())
    }

    /// Create a TCP client with custom configuration.
    #[must_use]
    pub fn with_config(config: TcpClientConfig) -> Self {
        Self { config }
    }

    /// Connect to an address and return a dedicated connection.
    pub async fn connect(&self, addr: SocketAddr) -> Result<TcpConnection, TransportError> {
        let connector = Connector::default();
        let io = connector
            .connect(Connect::new(addr))
            .await
            .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?;

        Ok(TcpConnection::new(io.seal().into(), self.config.max_frame_size))
    }
}

impl Default for TcpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// A single TCP connection.
///
/// This type is designed for single-threaded use within ntex's worker model.
/// It does NOT implement the core `Transport` trait (which requires Send+Sync)
/// because ntex uses Rc-based IO types.
///
/// Use `Rc<TcpConnection>` if you need to share within a single worker.
pub struct TcpConnection {
    io: Rc<RefCell<IoBoxed>>,
    codec: NimbusCodec,
    mux: Rc<Multiplexer>,
}

impl TcpConnection {
    /// Create a new TCP connection from an IO handle.
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
            .map_err(|e| TransportError::Io(std::sync::Arc::new(std::io::Error::other(format!("{}", e)))))?;

        // Write to buffer and flush
        io.with_write_buf(|write_buf| {
            write_buf.extend_from_slice(&buf);
            Ok::<_, std::io::Error>(())
        })
        .map_err(|e| TransportError::Io(std::sync::Arc::new(e)))?;

        io.flush(true)
            .await
            .map_err(|e| TransportError::Io(std::sync::Arc::new(std::io::Error::other(format!("{}", e)))))?;

        Ok(())
    }

    /// Receive a raw frame from the connection.
    pub async fn recv(&self) -> Result<AlignedVec, TransportError> {
        let io = self.io.borrow();
        let codec = &self.codec;

        loop {
            // Try to decode from existing buffer
            let result: Result<Option<AlignedVec>, std::io::Error> = io.with_read_buf(|read_buf| {
                let mut buf = BytesMut::from(read_buf.as_ref());
                match codec.decode(&mut buf) {
                    Ok(Some(frame)) => {
                        // Update the read position
                        let consumed = read_buf.len() - buf.len();
                        read_buf.advance(consumed);
                        Ok(Some(frame))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(std::io::Error::other(format!("{}", e))),
                }
            });

            match result {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {
                    // Need more data, wait for read
                    io.read_ready()
                        .await
                        .map_err(|e| TransportError::Io(std::sync::Arc::new(std::io::Error::other(format!("{}", e)))))?;

                    if io.is_closed() {
                        return Err(TransportError::ConnectionClosed);
                    }
                }
                Err(e) => return Err(TransportError::Io(std::sync::Arc::new(e))),
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
        let timeout = ctx.remaining().unwrap_or(Duration::from_secs(30));

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(NimbusError::Cancelled),
            Err(_) => {
                self.mux.cancel(request_id);
                Err(NimbusError::Timeout(timeout))
            }
        }
    }
}

/// Configuration for TCP server.
#[derive(Debug, Clone)]
pub struct TcpServerConfig {
    /// Address to bind to.
    pub bind_addr: String,

    /// Number of worker threads.
    pub workers: usize,

    /// Maximum concurrent connections.
    pub max_connections: usize,

    /// Connection backlog.
    pub backlog: usize,

    /// Maximum frame size.
    pub max_frame_size: usize,

    /// Keepalive timeout.
    pub keepalive_timeout: Duration,
}

impl Default for TcpServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:9000".to_string(),
            workers: num_cpus::get(),
            max_connections: 10000,
            backlog: 2048,
            max_frame_size: 16 * 1024 * 1024,
            keepalive_timeout: Duration::from_secs(60),
        }
    }
}

/// TCP server for accepting RPC connections.
pub struct TcpServer<H> {
    config: TcpServerConfig,
    handler: Arc<H>,
}

impl<H> TcpServer<H>
where
    H: RequestHandler + Send + Sync + 'static,
{
    /// Create a new TCP server with the given handler.
    pub fn new(handler: H) -> Self {
        Self {
            config: TcpServerConfig::default(),
            handler: Arc::new(handler),
        }
    }

    /// Create a server with custom configuration.
    pub fn with_config(config: TcpServerConfig, handler: H) -> Self {
        Self {
            config,
            handler: Arc::new(handler),
        }
    }

    /// Run the server.
    pub async fn run(self) -> std::io::Result<()> {
        let handler = self.handler.clone();
        let max_frame_size = self.config.max_frame_size;

        ntex::server::build()
            .workers(self.config.workers)
            .maxconn(self.config.max_connections)
            .backlog(self.config.backlog as i32)
            .bind("nimbus-tcp", &self.config.bind_addr, move |_| {
                let handler = handler.clone();
                fn_service(move |io: Io<_>| {
                    let handler = handler.clone();
                    async move {
                        // Each worker gets its own Rc wrapper around the shared Arc handler
                        let local_handler = Rc::new(handler);
                        handle_connection(io.seal().into(), local_handler, max_frame_size).await;
                        Ok::<_, std::io::Error>(())
                    }
                })
            })?
            .run()
            .await
    }
}

/// Trait for handling incoming RPC requests.
pub trait RequestHandler {
    /// Handle an incoming request.
    fn handle(
        &self,
        request: AlignedVec,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, NimbusError>>;
}

async fn handle_connection<H>(io: IoBoxed, handler: Rc<Arc<H>>, max_frame_size: usize)
where
    H: RequestHandler + Send + Sync + 'static,
{
    let codec = NimbusCodec::with_max_frame_size(max_frame_size);

    tracing::debug!("Connection established");

    loop {
        // Wait for data to be available
        if io.read_ready().await.is_err() || io.is_closed() {
            tracing::debug!("Connection closed");
            break;
        }

        // Try to decode frames from the read buffer
        let frame_result: Result<Option<AlignedVec>, std::io::Error> = io.with_read_buf(|read_buf| {
            let mut buf = BytesMut::from(read_buf.as_ref());
            match codec.decode(&mut buf) {
                Ok(Some(frame)) => {
                    let consumed = read_buf.len() - buf.len();
                    read_buf.advance(consumed);
                    Ok(Some(frame))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(std::io::Error::other(format!("{}", e))),
            }
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
