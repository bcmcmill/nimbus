//! TLS support for nimbus transport.
//!
//! This module provides TLS configuration and transport types using Rustls.
//!
//! ## Example
//!
//! ```rust,ignore
//! use nimbus_transport::{TlsClient, TlsClientConfig, TlsServerConfig};
//!
//! // Client with default root certificates
//! let client_config = TlsClientConfig::new();
//! let client = TlsClient::new(client_config);
//! let conn = client.connect("example.com:443").await?;
//!
//! // Server with certificate files
//! let server_config = TlsServerConfig::load_from_pem("cert.pem", "key.pem")?;
//! let server = TlsServer::new(server_config, handler);
//! server.run().await?;
//! ```

use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore, ServerConfig};

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
