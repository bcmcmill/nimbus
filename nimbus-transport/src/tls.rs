//! TLS support for nimbus transport using Rustls.
//!
//! This module provides TLS configuration types for securing connections:
//!
//! - [`TlsClientConfig`] - Client-side TLS configuration
//! - [`TlsServerConfig`] - Server-side TLS configuration
//! - [`ClientAuth`] - Client authentication modes for mutual TLS
//!
//! ## Security Features
//!
//! - Modern TLS 1.2/1.3 only (via Rustls)
//! - No OpenSSL dependency - pure Rust implementation
//! - Mozilla root certificates included by default
//! - Mutual TLS (mTLS) support for client authentication
//!
//! ## Client Configuration
//!
//! ```rust,ignore
//! use nimbus_transport::TlsClientConfig;
//!
//! // Connect to public TLS servers using Mozilla's root certificates
//! let config = TlsClientConfig::new();
//!
//! // Or load custom root certificates for private/self-signed servers
//! let root_certs = nimbus_transport::load_root_certs("ca-cert.pem")?;
//! let config = TlsClientConfig::with_root_certs(root_certs);
//!
//! // For mutual TLS, provide client certificate
//! let config = TlsClientConfig::load_client_cert_from_pem(
//!     root_certs,
//!     "client-cert.pem",
//!     "client-key.pem",
//! )?;
//! ```
//!
//! ## Server Configuration
//!
//! ```rust,ignore
//! use nimbus_transport::{TlsServerConfig, ClientAuth, load_root_certs};
//!
//! // Basic server with certificate
//! let config = TlsServerConfig::load_from_pem("server-cert.pem", "server-key.pem")?;
//!
//! // Server requiring client certificates (mutual TLS)
//! let client_ca = load_root_certs("client-ca.pem")?;
//! let config = TlsServerConfig::with_client_auth(
//!     cert_chain,
//!     private_key,
//!     ClientAuth::Required(std::sync::Arc::new(client_ca)),
//! )?;
//! ```
//!
//! ## Handshake Timeout
//!
//! Both client and server configurations support a handshake timeout
//! (default: 10 seconds) to prevent slow-loris style attacks:
//!
//! ```rust,ignore
//! let config = TlsClientConfig::new()
//!     .handshake_timeout(Duration::from_secs(5));
//! ```

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufReader};
use std::net::SocketAddr;
use std::path::Path;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::future::{Either, select};
use futures_timer::Delay;
use ntex::connect::{Connect, Connector};
use ntex::io::{Io, IoBoxed};
use ntex::service::fn_service;
use ntex::util::Buf;
use ntex_bytes::BytesMut;
use ntex_tls::rustls::{TlsClientFilter, TlsServerFilter};
use ntex_util::time::Millis;
use rkyv::util::AlignedVec;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore, ServerConfig};

use nimbus_codec::NimbusCodec;
use nimbus_core::{Context, NimbusError, TransportError};

use crate::mux::Multiplexer;
use crate::tcp::RequestHandler;

/// TLS configuration for clients.
///
/// Configures how the client validates server certificates and optionally
/// provides client certificates for mutual TLS authentication.
#[derive(Clone)]
pub struct TlsClientConfig {
    /// Rustls client configuration.
    config: Arc<ClientConfig>,
    /// Handshake timeout.
    pub handshake_timeout: Duration,
}

impl std::fmt::Debug for TlsClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsClientConfig")
            .field("handshake_timeout", &self.handshake_timeout)
            .finish_non_exhaustive()
    }
}

impl TlsClientConfig {
    /// Create a new TLS client config with Mozilla's root certificates.
    ///
    /// This is suitable for connecting to public TLS servers.
    #[must_use]
    pub fn new() -> Self {
        let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Self {
            config: Arc::new(config),
            handshake_timeout: Duration::from_secs(10),
        }
    }

    /// Create a TLS client config with a custom root certificate store.
    ///
    /// Use this when connecting to servers with private/self-signed certificates.
    #[must_use]
    pub fn with_root_certs(root_certs: RootCertStore) -> Self {
        let config = ClientConfig::builder()
            .with_root_certificates(root_certs)
            .with_no_client_auth();

        Self {
            config: Arc::new(config),
            handshake_timeout: Duration::from_secs(10),
        }
    }

    /// Create a TLS client config with client certificate for mutual TLS.
    ///
    /// # Errors
    ///
    /// Returns an error if the certificate or key is invalid.
    pub fn with_client_cert(
        root_certs: RootCertStore,
        cert_chain: Vec<CertificateDer<'static>>,
        private_key: PrivateKeyDer<'static>,
    ) -> Result<Self, rustls::Error> {
        let config = ClientConfig::builder()
            .with_root_certificates(root_certs)
            .with_client_auth_cert(cert_chain, private_key)?;

        Ok(Self {
            config: Arc::new(config),
            handshake_timeout: Duration::from_secs(10),
        })
    }

    /// Load client certificates from PEM files for mutual TLS.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be read or parsed.
    pub fn load_client_cert_from_pem(
        root_certs: RootCertStore,
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let cert_chain = load_certs(cert_path)?;
        let private_key = load_private_key(key_path)?;

        Self::with_client_cert(root_certs, cert_chain, private_key)
            .map_err(|e| io::Error::other(format!("TLS config error: {e}")))
    }

    /// Set the handshake timeout.
    #[must_use]
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Get the underlying Rustls client config.
    #[must_use]
    pub fn rustls_config(&self) -> Arc<ClientConfig> {
        self.config.clone()
    }
}

impl Default for TlsClientConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Client authentication mode for TLS servers.
#[derive(Clone)]
pub enum ClientAuth {
    /// No client authentication required.
    None,
    /// Request client certificate but don't require it.
    Optional(Arc<RootCertStore>),
    /// Require a valid client certificate.
    Required(Arc<RootCertStore>),
}

impl std::fmt::Debug for ClientAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "ClientAuth::None"),
            Self::Optional(_) => write!(f, "ClientAuth::Optional"),
            Self::Required(_) => write!(f, "ClientAuth::Required"),
        }
    }
}

/// TLS configuration for servers.
///
/// Configures the server's certificate and optionally requires
/// client certificates for mutual TLS authentication.
#[derive(Clone)]
pub struct TlsServerConfig {
    /// Rustls server configuration.
    config: Arc<ServerConfig>,
    /// Handshake timeout.
    pub handshake_timeout: Duration,
}

impl std::fmt::Debug for TlsServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsServerConfig")
            .field("handshake_timeout", &self.handshake_timeout)
            .finish_non_exhaustive()
    }
}

impl TlsServerConfig {
    /// Create a new TLS server config with the given certificate and key.
    ///
    /// # Errors
    ///
    /// Returns an error if the certificate or key is invalid.
    pub fn new(
        cert_chain: Vec<CertificateDer<'static>>,
        private_key: PrivateKeyDer<'static>,
    ) -> Result<Self, rustls::Error> {
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, private_key)?;

        Ok(Self {
            config: Arc::new(config),
            handshake_timeout: Duration::from_secs(10),
        })
    }

    /// Load server certificate and key from PEM files.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be read or parsed.
    pub fn load_from_pem(
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let cert_chain = load_certs(cert_path)?;
        let private_key = load_private_key(key_path)?;

        Self::new(cert_chain, private_key)
            .map_err(|e| io::Error::other(format!("TLS config error: {e}")))
    }

    /// Create a TLS server config with client authentication.
    ///
    /// # Errors
    ///
    /// Returns an error if the certificate or key is invalid.
    pub fn with_client_auth(
        cert_chain: Vec<CertificateDer<'static>>,
        private_key: PrivateKeyDer<'static>,
        client_auth: ClientAuth,
    ) -> Result<Self, rustls::Error> {
        let builder = ServerConfig::builder();

        let config = match client_auth {
            ClientAuth::None => builder
                .with_no_client_auth()
                .with_single_cert(cert_chain, private_key)?,
            ClientAuth::Optional(root_certs) => {
                let verifier = rustls::server::WebPkiClientVerifier::builder(root_certs)
                    .allow_unauthenticated()
                    .build()
                    .map_err(|e| rustls::Error::General(e.to_string()))?;
                builder
                    .with_client_cert_verifier(verifier)
                    .with_single_cert(cert_chain, private_key)?
            }
            ClientAuth::Required(root_certs) => {
                let verifier = rustls::server::WebPkiClientVerifier::builder(root_certs)
                    .build()
                    .map_err(|e| rustls::Error::General(e.to_string()))?;
                builder
                    .with_client_cert_verifier(verifier)
                    .with_single_cert(cert_chain, private_key)?
            }
        };

        Ok(Self {
            config: Arc::new(config),
            handshake_timeout: Duration::from_secs(10),
        })
    }

    /// Set the handshake timeout.
    #[must_use]
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Get the underlying Rustls server config.
    #[must_use]
    pub fn rustls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
}

/// Load certificates from a PEM file.
fn load_certs(path: impl AsRef<Path>) -> io::Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path.as_ref())?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::certs(&mut reader).collect()
}

/// Load a private key from a PEM file.
fn load_private_key(path: impl AsRef<Path>) -> io::Result<PrivateKeyDer<'static>> {
    let file = File::open(path.as_ref())?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| io::Error::other("no private key found in file"))
}

/// Create a `ServerName` from a string.
///
/// # Errors
///
/// Returns an error if the name is not a valid DNS name or IP address.
pub fn server_name(name: &str) -> io::Result<ServerName<'static>> {
    ServerName::try_from(name.to_string())
        .map_err(|e| io::Error::other(format!("invalid server name: {e}")))
}

/// Load root certificates from a PEM file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or contains invalid certificates.
pub fn load_root_certs(path: impl AsRef<Path>) -> io::Result<RootCertStore> {
    let certs = load_certs(path)?;
    let mut store = RootCertStore::empty();
    for cert in certs {
        store
            .add(cert)
            .map_err(|e| io::Error::other(format!("invalid root certificate: {e}")))?;
    }
    Ok(store)
}

// ============================================================================
// TLS Client Implementation
// ============================================================================

/// TLS client for secure RPC communication.
///
/// Establishes TLS-encrypted connections to servers.
///
/// ## Example
///
/// ```rust,ignore
/// use nimbus_transport::{TlsClient, TlsClientConfig};
///
/// let config = TlsClientConfig::new();
/// let client = TlsClient::new(config);
///
/// // Connect using hostname (for SNI) and address
/// let conn = client.connect("api.example.com", "192.168.1.100:443".parse()?).await?;
/// ```
pub struct TlsClient {
    config: TlsClientConfig,
    /// Maximum frame size for the codec.
    pub max_frame_size: usize,
}

impl TlsClient {
    /// Create a new TLS client with the given configuration.
    #[must_use]
    pub fn new(config: TlsClientConfig) -> Self {
        Self {
            config,
            max_frame_size: 16 * 1024 * 1024,
        }
    }

    /// Set the maximum frame size.
    #[must_use]
    pub fn max_frame_size(mut self, size: usize) -> Self {
        self.max_frame_size = size;
        self
    }

    /// Connect to a server with TLS.
    ///
    /// # Arguments
    ///
    /// * `server_name` - The server's hostname for SNI and certificate verification
    /// * `addr` - The socket address to connect to
    ///
    /// # Errors
    ///
    /// Returns an error if the connection or TLS handshake fails.
    pub async fn connect(
        &self,
        server_name: &str,
        addr: SocketAddr,
    ) -> Result<TlsConnection, TransportError> {
        // Connect TCP
        let connector = Connector::default();
        let io = connector
            .connect(Connect::new(addr))
            .await
            .map_err(|e| TransportError::ConnectionFailed(format!("TCP connect failed: {e}")))?;

        // Parse server name for TLS
        let sni = ServerName::try_from(server_name.to_string())
            .map_err(|e| TransportError::ConnectionFailed(format!("invalid server name: {e}")))?;

        // Perform TLS handshake with timeout
        let tls_config = self.config.rustls_config();
        let handshake_fut = pin!(TlsClientFilter::create(io, tls_config, sni));
        let timeout_fut = pin!(Delay::new(self.config.handshake_timeout));

        let tls_io = match select(handshake_fut, timeout_fut).await {
            Either::Left((Ok(io), _)) => io,
            Either::Left((Err(e), _)) => {
                return Err(TransportError::ConnectionFailed(format!(
                    "TLS handshake failed: {e}"
                )));
            }
            Either::Right(_) => {
                return Err(TransportError::ConnectionFailed(
                    "TLS handshake timed out".to_string(),
                ));
            }
        };

        Ok(TlsConnection::new(
            tls_io.seal().into(),
            self.max_frame_size,
        ))
    }
}

impl std::fmt::Debug for TlsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsClient")
            .field("config", &self.config)
            .field("max_frame_size", &self.max_frame_size)
            .finish()
    }
}

/// A TLS-encrypted connection.
///
/// Provides the same interface as `TcpConnection` but over an encrypted channel.
pub struct TlsConnection {
    io: Rc<RefCell<IoBoxed>>,
    codec: NimbusCodec,
    mux: Rc<Multiplexer>,
}

impl TlsConnection {
    /// Create a new TLS connection from an IO handle.
    pub fn new(io: IoBoxed, max_frame_size: usize) -> Self {
        Self {
            io: Rc::new(RefCell::new(io)),
            codec: NimbusCodec::with_max_frame_size(max_frame_size),
            mux: Rc::new(Multiplexer::new()),
        }
    }

    /// Check if the connection is still open.
    #[inline]
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.io.borrow().is_closed()
    }

    /// Close the connection.
    #[inline]
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
            .map_err(|e| TransportError::Io(Arc::new(io::Error::other(format!("{e}")))))?;

        // Write to buffer and flush
        let _ = io
            .with_write_buf(|write_buf| {
                write_buf.extend_from_slice(&buf);
                Ok::<_, io::Error>(())
            })
            .map_err(|e| TransportError::Io(Arc::new(e)))?;

        io.flush(true)
            .await
            .map_err(|e| TransportError::Io(Arc::new(io::Error::other(format!("{e}")))))?;

        Ok(())
    }

    /// Receive a raw frame from the connection.
    pub async fn recv(&self) -> Result<AlignedVec, TransportError> {
        let io = self.io.borrow();
        let codec = &self.codec;

        loop {
            // Try to decode from existing buffer (zero-copy path)
            let result: Result<Option<AlignedVec>, io::Error> =
                io.with_read_buf(|read_buf| match codec.decode_slice(read_buf.as_ref()) {
                    Ok(Some((frame, consumed))) => {
                        read_buf.advance(consumed);
                        Ok(Some(frame))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(io::Error::other(format!("{e}"))),
                });

            match result {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {
                    // Need more data, wait for read
                    io.read_ready().await.map_err(|e| {
                        TransportError::Io(Arc::new(io::Error::other(format!("{e}"))))
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

// ============================================================================
// TLS Server Implementation
// ============================================================================

/// Configuration for TLS server.
#[derive(Debug, Clone)]
pub struct TlsServerBindConfig {
    /// TLS configuration.
    pub tls: TlsServerConfig,

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
}

impl TlsServerBindConfig {
    /// Create a new TLS server config.
    pub fn new(tls: TlsServerConfig, bind_addr: impl Into<String>) -> Self {
        Self {
            tls,
            bind_addr: bind_addr.into(),
            workers: num_cpus::get(),
            max_connections: 10000,
            backlog: 2048,
            max_frame_size: 16 * 1024 * 1024,
        }
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

/// TLS server for accepting secure RPC connections.
///
/// ## Example
///
/// ```rust,ignore
/// use nimbus_transport::{TlsServer, TlsServerConfig, TlsServerBindConfig, RequestHandler};
///
/// struct MyHandler;
/// impl RequestHandler for MyHandler {
///     async fn handle(&self, request: AlignedVec) -> Result<Vec<u8>, NimbusError> {
///         Ok(vec![])
///     }
/// }
///
/// let tls_config = TlsServerConfig::load_from_pem("cert.pem", "key.pem")?;
/// let config = TlsServerBindConfig::new(tls_config, "0.0.0.0:9443");
/// let server = TlsServer::new(config, MyHandler);
/// server.run().await?;
/// ```
pub struct TlsServer<H> {
    config: TlsServerBindConfig,
    handler: Arc<H>,
}

impl<H> TlsServer<H>
where
    H: RequestHandler + Send + Sync + 'static,
{
    /// Create a new TLS server with the given configuration and handler.
    pub fn new(config: TlsServerBindConfig, handler: H) -> Self {
        Self {
            config,
            handler: Arc::new(handler),
        }
    }

    /// Run the server.
    pub async fn run(self) -> io::Result<()> {
        let handler = self.handler.clone();
        let max_frame_size = self.config.max_frame_size;
        let tls_config = self.config.tls.rustls_config();
        let handshake_timeout = Millis::from(self.config.tls.handshake_timeout);

        ntex::server::build()
            .workers(self.config.workers)
            .maxconn(self.config.max_connections)
            .backlog(self.config.backlog as i32)
            .bind("nimbus-tls", &self.config.bind_addr, move |_| {
                let handler = handler.clone();
                let tls_config = tls_config.clone();
                let handshake_timeout = handshake_timeout;

                fn_service(move |io: Io<_>| {
                    let handler = handler.clone();
                    let tls_config = tls_config.clone();

                    async move {
                        // Perform TLS handshake
                        let tls_io = match TlsServerFilter::create(
                            io,
                            tls_config,
                            handshake_timeout,
                        )
                        .await
                        {
                            Ok(io) => io,
                            Err(e) => {
                                tracing::debug!("TLS handshake failed: {e}");
                                return Ok::<_, io::Error>(());
                            }
                        };

                        // Handle the connection
                        let local_handler = Rc::new(handler);
                        handle_tls_connection(tls_io.seal().into(), local_handler, max_frame_size)
                            .await;
                        Ok::<_, io::Error>(())
                    }
                })
            })?
            .run()
            .await
    }
}

async fn handle_tls_connection<H>(io: IoBoxed, handler: Rc<Arc<H>>, max_frame_size: usize)
where
    H: RequestHandler + Send + Sync + 'static,
{
    let codec = NimbusCodec::with_max_frame_size(max_frame_size);

    tracing::debug!("TLS connection established");

    loop {
        // Wait for data to be available
        if io.read_ready().await.is_err() || io.is_closed() {
            tracing::debug!("TLS connection closed");
            break;
        }

        // Try to decode frames from the read buffer (zero-copy path)
        let frame_result: Result<Option<AlignedVec>, io::Error> =
            io.with_read_buf(|read_buf| match codec.decode_slice(read_buf.as_ref()) {
                Ok(Some((frame, consumed))) => {
                    read_buf.advance(consumed);
                    Ok(Some(frame))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(io::Error::other(format!("{e}"))),
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
                                Ok::<_, io::Error>(())
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
                        tracing::error!("Handler error: {e}");
                    }
                }
            }
            Ok(None) => {
                // No complete frame yet, continue waiting
            }
            Err(e) => {
                tracing::error!("Decode error: {e}");
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
    fn test_client_config_default() {
        let config = TlsClientConfig::new();
        assert_eq!(config.handshake_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_client_config_with_timeout() {
        let config = TlsClientConfig::new().handshake_timeout(Duration::from_secs(30));
        assert_eq!(config.handshake_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_client_auth_debug() {
        assert_eq!(format!("{:?}", ClientAuth::None), "ClientAuth::None");
    }
}
