//! Error types for the Nimbus RPC framework.

use std::time::Duration;

/// Main error type for Nimbus operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum NimbusError {
    /// Transport-level error (connection, IO, etc.)
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    /// Codec error (serialization/deserialization)
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),

    /// Service-level error returned by the RPC handler
    #[error("service error [{code}]: {message}")]
    Service {
        /// Error code for programmatic handling
        code: u32,
        /// Human-readable error message
        message: String,
    },

    /// Request timed out
    #[error("timeout after {0:?}")]
    Timeout(Duration),

    /// Request was cancelled
    #[error("request cancelled")]
    Cancelled,

    /// Method not found on the service
    #[error("method not found: {0}")]
    MethodNotFound(String),

    /// Service not found
    #[error("service not found: {0}")]
    ServiceNotFound(String),

    /// Invalid request format
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

/// Transport-level errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TransportError {
    /// IO error from the underlying transport
    #[error("io error: {0}")]
    Io(std::sync::Arc<std::io::Error>),

    /// Connection was closed unexpectedly
    #[error("connection closed")]
    ConnectionClosed,

    /// Failed to connect to the remote endpoint
    #[error("connection failed: {0}")]
    ConnectionFailed(String),

    /// Connection pool exhausted
    #[error("no available connections")]
    PoolExhausted,

    /// DNS resolution failed
    #[error("dns resolution failed: {0}")]
    DnsResolutionFailed(String),

    /// TLS handshake failed
    #[error("tls error: {0}")]
    Tls(String),

    /// Protocol violation
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// Codec errors for serialization and framing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CodecError {
    /// Frame size exceeds maximum allowed
    #[error("frame too large: {size} bytes (max: {max})")]
    FrameTooLarge {
        /// Actual frame size
        size: usize,
        /// Maximum allowed size
        max: usize,
    },

    /// Invalid frame format
    #[error("invalid frame: {0}")]
    InvalidFrame(String),

    /// rkyv serialization failed
    #[error("serialization error: {0}")]
    Serialization(String),

    /// rkyv deserialization/validation failed
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Buffer alignment error
    #[error("alignment error: buffer not properly aligned")]
    Alignment,
}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(std::sync::Arc::new(e))
    }
}

impl NimbusError {
    /// Create a service error with code and message.
    #[must_use]
    pub fn service(code: u32, message: impl Into<String>) -> Self {
        Self::Service {
            code,
            message: message.into(),
        }
    }

    /// Check if this error is retryable.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(TransportError::Io(_)) => true,
            Self::Transport(TransportError::ConnectionClosed) => true,
            Self::Transport(TransportError::PoolExhausted) => true,
            Self::Timeout(_) => true,
            _ => false,
        }
    }

    /// Check if this error indicates the connection should be closed.
    #[must_use]
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self,
            Self::Transport(TransportError::ConnectionClosed)
                | Self::Transport(TransportError::Io(_))
                | Self::Transport(TransportError::Protocol(_))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = NimbusError::service(404, "user not found");
        assert_eq!(err.to_string(), "service error [404]: user not found");
    }

    #[test]
    fn test_retryable() {
        assert!(NimbusError::Timeout(Duration::from_secs(1)).is_retryable());
        assert!(!NimbusError::Cancelled.is_retryable());
    }
}
