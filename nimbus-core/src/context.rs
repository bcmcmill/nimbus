//! Request context for RPC calls.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Global request ID counter for unique IDs.
static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique request ID.
fn next_request_id() -> u64 {
    REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Distributed tracing identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TraceId(pub [u8; 16]);

impl TraceId {
    /// Create a new random trace ID.
    #[must_use]
    pub fn new() -> Self {
        let mut bytes = [0u8; 16];
        // Simple random generation - in production, use proper random source
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let nanos = now.as_nanos() as u64;
        let id = next_request_id();
        bytes[..8].copy_from_slice(&nanos.to_le_bytes());
        bytes[8..].copy_from_slice(&id.to_le_bytes());
        Self(bytes)
    }

    /// Create from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Get the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TraceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Metadata key-value pairs for request context.
pub type Metadata = HashMap<String, String>;

/// Request context carrying metadata, deadlines, and tracing information.
///
/// Every RPC call carries a `Context` that provides:
/// - Unique request identification
/// - Deadline/timeout propagation
/// - Distributed tracing correlation
/// - Custom metadata (similar to HTTP headers)
#[derive(Debug, Clone)]
pub struct Context {
    /// Unique identifier for this request.
    pub request_id: u64,

    /// Absolute deadline for this request.
    /// If `None`, no deadline is enforced.
    pub deadline: Option<Instant>,

    /// Custom key-value metadata.
    pub metadata: Metadata,

    /// Distributed tracing identifier.
    pub trace_id: Option<TraceId>,

    /// Parent span ID for tracing.
    pub parent_span_id: Option<u64>,
}

impl Context {
    /// Create a new context with a unique request ID.
    #[must_use]
    pub fn new() -> Self {
        Self {
            request_id: next_request_id(),
            deadline: None,
            metadata: HashMap::new(),
            trace_id: None,
            parent_span_id: None,
        }
    }

    /// Create a context with a timeout from now.
    #[must_use]
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            request_id: next_request_id(),
            deadline: Some(Instant::now() + timeout),
            metadata: HashMap::new(),
            trace_id: None,
            parent_span_id: None,
        }
    }

    /// Create a context with an absolute deadline.
    #[must_use]
    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            request_id: next_request_id(),
            deadline: Some(deadline),
            metadata: HashMap::new(),
            trace_id: None,
            parent_span_id: None,
        }
    }

    /// Set the deadline for this context.
    #[must_use]
    pub fn deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Set a timeout from now.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.deadline = Some(Instant::now() + timeout);
        self
    }

    /// Add metadata to the context.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Set the trace ID.
    #[must_use]
    pub fn with_trace_id(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Enable tracing with a new trace ID.
    #[must_use]
    pub fn with_tracing(mut self) -> Self {
        self.trace_id = Some(TraceId::new());
        self
    }

    /// Get remaining time until deadline.
    /// Returns `None` if no deadline is set or deadline has passed.
    #[must_use]
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .and_then(|d| d.checked_duration_since(Instant::now()))
    }

    /// Check if the deadline has passed.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    /// Get a metadata value.
    #[must_use]
    pub fn get_metadata(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }

    /// Create a child context for nested calls.
    /// Inherits deadline, trace ID, and metadata.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            request_id: next_request_id(),
            deadline: self.deadline,
            metadata: self.metadata.clone(),
            trace_id: self.trace_id.clone(),
            parent_span_id: Some(self.request_id),
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = Context::new();
        assert!(ctx.request_id > 0);
        assert!(ctx.deadline.is_none());
    }

    #[test]
    fn test_context_with_timeout() {
        let ctx = Context::with_timeout(Duration::from_secs(10));
        assert!(ctx.deadline.is_some());
        assert!(!ctx.is_expired());
        assert!(ctx.remaining().is_some());
    }

    #[test]
    fn test_context_metadata() {
        let ctx = Context::new()
            .with_metadata("user-id", "123")
            .with_metadata("request-type", "query");

        assert_eq!(ctx.get_metadata("user-id"), Some("123"));
        assert_eq!(ctx.get_metadata("request-type"), Some("query"));
        assert_eq!(ctx.get_metadata("nonexistent"), None);
    }

    #[test]
    fn test_child_context() {
        let parent = Context::new()
            .with_tracing()
            .with_metadata("inherited", "yes");

        let child = parent.child();

        assert_ne!(child.request_id, parent.request_id);
        assert_eq!(child.trace_id, parent.trace_id);
        assert_eq!(child.parent_span_id, Some(parent.request_id));
        assert_eq!(child.get_metadata("inherited"), Some("yes"));
    }

    #[test]
    fn test_trace_id_display() {
        let trace_id = TraceId::from_bytes([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ]);
        assert_eq!(trace_id.to_string(), "0123456789abcdef0123456789abcdef");
    }
}
