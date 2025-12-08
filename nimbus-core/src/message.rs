//! RPC message envelope types.
//!
//! These types define the wire format for Nimbus RPC messages,
//! serialized using rkyv for zero-copy deserialization.

use rkyv::{Archive, Deserialize, Serialize};

/// RPC message envelope containing routing and payload information.
///
/// This is the top-level message format sent over the wire.
/// All fields are rkyv-serializable for zero-copy access.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rkyv(derive(Debug))]
pub struct RpcEnvelope {
    /// Unique request ID for correlation.
    pub request_id: u64,

    /// Type of message (request, response, stream, etc.)
    pub message_type: MessageType,

    /// Target service name.
    pub service: String,

    /// Target method name.
    pub method: String,

    /// Serialized payload (service-specific request/response).
    pub payload: Vec<u8>,
}

/// Type of RPC message.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[rkyv(derive(Debug))]
#[repr(u8)]
pub enum MessageType {
    /// Unary request expecting a response.
    Request = 1,

    /// Response to a request.
    Response = 2,

    /// Error response.
    Error = 3,

    /// Start of a stream.
    StreamStart = 4,

    /// Data item in a stream.
    StreamData = 5,

    /// End of a stream.
    StreamEnd = 6,

    /// Stream cancelled by sender.
    StreamCancel = 7,

    /// Ping for keepalive.
    Ping = 8,

    /// Pong response to ping.
    Pong = 9,
}

/// Error information sent in error responses.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[rkyv(derive(Debug))]
pub struct RpcError {
    /// Error code for programmatic handling.
    pub code: u32,

    /// Human-readable error message.
    pub message: String,

    /// Optional additional details.
    pub details: Option<Vec<u8>>,
}

impl RpcEnvelope {
    /// Create a new request envelope.
    #[must_use]
    pub fn request(
        request_id: u64,
        service: impl Into<String>,
        method: impl Into<String>,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            request_id,
            message_type: MessageType::Request,
            service: service.into(),
            method: method.into(),
            payload,
        }
    }

    /// Create a response envelope.
    #[must_use]
    pub fn response(request_id: u64, service: impl Into<String>, method: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            request_id,
            message_type: MessageType::Response,
            service: service.into(),
            method: method.into(),
            payload,
        }
    }

    /// Create an error response envelope.
    #[must_use]
    pub fn error(
        request_id: u64,
        service: impl Into<String>,
        method: impl Into<String>,
        error: RpcError,
    ) -> Self {
        // Serialize the error into payload
        let payload = rkyv::to_bytes::<rkyv::rancor::Error>(&error)
            .map(|v| v.to_vec())
            .unwrap_or_default();

        Self {
            request_id,
            message_type: MessageType::Error,
            service: service.into(),
            method: method.into(),
            payload,
        }
    }

    /// Create a ping message.
    #[must_use]
    pub fn ping(request_id: u64) -> Self {
        Self {
            request_id,
            message_type: MessageType::Ping,
            service: String::new(),
            method: String::new(),
            payload: Vec::new(),
        }
    }

    /// Create a pong response.
    #[must_use]
    pub fn pong(request_id: u64) -> Self {
        Self {
            request_id,
            message_type: MessageType::Pong,
            service: String::new(),
            method: String::new(),
            payload: Vec::new(),
        }
    }

    /// Create a stream start message.
    #[must_use]
    pub fn stream_start(
        request_id: u64,
        service: impl Into<String>,
        method: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            message_type: MessageType::StreamStart,
            service: service.into(),
            method: method.into(),
            payload: Vec::new(),
        }
    }

    /// Create a stream data message.
    #[must_use]
    pub fn stream_data(
        request_id: u64,
        service: impl Into<String>,
        method: impl Into<String>,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            request_id,
            message_type: MessageType::StreamData,
            service: service.into(),
            method: method.into(),
            payload,
        }
    }

    /// Create a stream end message.
    #[must_use]
    pub fn stream_end(request_id: u64, service: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            request_id,
            message_type: MessageType::StreamEnd,
            service: service.into(),
            method: method.into(),
            payload: Vec::new(),
        }
    }

    /// Check if this is a request message.
    #[must_use]
    pub fn is_request(&self) -> bool {
        matches!(self.message_type, MessageType::Request)
    }

    /// Check if this is a response message.
    #[must_use]
    pub fn is_response(&self) -> bool {
        matches!(self.message_type, MessageType::Response | MessageType::Error)
    }

    /// Check if this is a streaming message.
    #[must_use]
    pub fn is_stream(&self) -> bool {
        matches!(
            self.message_type,
            MessageType::StreamStart
                | MessageType::StreamData
                | MessageType::StreamEnd
                | MessageType::StreamCancel
        )
    }
}

impl RpcError {
    /// Create a new RPC error.
    #[must_use]
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// Create an error with details.
    #[must_use]
    pub fn with_details(code: u32, message: impl Into<String>, details: Vec<u8>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details),
        }
    }

    // Common error codes
    /// Error code for cancelled requests.
    pub const CANCELLED: u32 = 1;
    /// Error code for unknown errors.
    pub const UNKNOWN: u32 = 2;
    /// Error code for invalid arguments.
    pub const INVALID_ARGUMENT: u32 = 3;
    /// Error code for deadline exceeded.
    pub const DEADLINE_EXCEEDED: u32 = 4;
    /// Error code for not found.
    pub const NOT_FOUND: u32 = 5;
    /// Error code for already exists.
    pub const ALREADY_EXISTS: u32 = 6;
    /// Error code for permission denied.
    pub const PERMISSION_DENIED: u32 = 7;
    /// Error code for resource exhausted.
    pub const RESOURCE_EXHAUSTED: u32 = 8;
    /// Error code for internal errors.
    pub const INTERNAL: u32 = 13;
    /// Error code for unavailable service.
    pub const UNAVAILABLE: u32 = 14;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rkyv::access;

    #[test]
    fn test_envelope_roundtrip() {
        let envelope = RpcEnvelope::request(
            42,
            "calculator",
            "add",
            vec![1, 2, 3, 4],
        );

        // Serialize
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).unwrap();

        // Zero-copy access
        let archived = access::<ArchivedRpcEnvelope, rkyv::rancor::Error>(&bytes).unwrap();

        assert_eq!(archived.request_id, 42);
        assert_eq!(archived.service.as_str(), "calculator");
        assert_eq!(archived.method.as_str(), "add");
        assert_eq!(archived.payload.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_message_types() {
        let req = RpcEnvelope::request(1, "svc", "method", vec![]);
        assert!(req.is_request());
        assert!(!req.is_response());
        assert!(!req.is_stream());

        let resp = RpcEnvelope::response(1, "svc", "method", vec![]);
        assert!(!resp.is_request());
        assert!(resp.is_response());

        let stream = RpcEnvelope::stream_data(1, "svc", "method", vec![]);
        assert!(stream.is_stream());
    }
}
