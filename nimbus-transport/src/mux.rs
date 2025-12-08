//! Request/response multiplexing for concurrent RPC calls.

use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use rkyv::util::AlignedVec;
use tokio::sync::oneshot;

use nimbus_core::{NimbusError, TransportError};

/// A pending request waiting for a response.
struct PendingRequest {
    sender: oneshot::Sender<Result<AlignedVec, NimbusError>>,
}

/// Multiplexer for correlating requests with responses.
///
/// This allows multiple concurrent RPC calls over a single connection
/// by assigning unique IDs to requests and routing responses back
/// to the appropriate caller.
///
/// ## Example
///
/// ```rust
/// use nimbus_transport::Multiplexer;
///
/// let mux = Multiplexer::new();
///
/// // Register a pending request
/// let (id, receiver) = mux.register();
///
/// // Later, dispatch the response
/// // mux.dispatch(id, response_data);
///
/// // The receiver will complete with the response
/// ```
pub struct Multiplexer {
    pending: DashMap<u64, PendingRequest>,
    next_id: AtomicU64,
}

impl Multiplexer {
    /// Create a new multiplexer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: DashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Register a new pending request.
    ///
    /// Returns the request ID and a receiver for the response.
    pub fn register(&self) -> (u64, oneshot::Receiver<Result<AlignedVec, NimbusError>>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();

        self.pending.insert(id, PendingRequest { sender: tx });

        (id, rx)
    }

    /// Dispatch a response to a pending request.
    ///
    /// Returns `true` if the response was delivered, `false` if no
    /// pending request with that ID was found (e.g., it timed out).
    pub fn dispatch(&self, request_id: u64, response: AlignedVec) -> bool {
        if let Some((_, pending)) = self.pending.remove(&request_id) {
            // Ignore send errors - receiver may have been dropped (timeout)
            let _ = pending.sender.send(Ok(response));
            true
        } else {
            tracing::warn!(request_id, "No pending request found for response");
            false
        }
    }

    /// Dispatch an error to a pending request.
    pub fn dispatch_error(&self, request_id: u64, error: NimbusError) -> bool {
        if let Some((_, pending)) = self.pending.remove(&request_id) {
            let _ = pending.sender.send(Err(error));
            true
        } else {
            false
        }
    }

    /// Cancel a pending request.
    ///
    /// This is called when a request times out or is explicitly cancelled.
    pub fn cancel(&self, request_id: u64) {
        if let Some((_, pending)) = self.pending.remove(&request_id) {
            let _ = pending.sender.send(Err(NimbusError::Cancelled));
        }
    }

    /// Cancel all pending requests (e.g., on connection close).
    pub fn cancel_all(&self) {
        // Collect keys first to avoid iterator invalidation
        let keys: Vec<u64> = self.pending.iter().map(|e| *e.key()).collect();

        let error = NimbusError::Transport(TransportError::ConnectionClosed);
        for id in keys {
            if let Some((_, pending)) = self.pending.remove(&id) {
                let _ = pending.sender.send(Err(error.clone()));
            }
        }
    }

    /// Get the number of pending requests.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Check if there are any pending requests.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

impl Default for Multiplexer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_dispatch() {
        let mux = Multiplexer::new();

        let (id, mut rx) = mux.register();
        assert!(mux.has_pending());
        assert_eq!(mux.pending_count(), 1);

        let response = AlignedVec::new();
        assert!(mux.dispatch(id, response));

        // Receiver should have the response
        let result = rx.try_recv();
        assert!(result.is_ok());
    }

    #[test]
    fn test_dispatch_unknown_id() {
        let mux = Multiplexer::new();
        let response = AlignedVec::new();

        // Dispatching to unknown ID should return false
        assert!(!mux.dispatch(999, response));
    }

    #[test]
    fn test_cancel() {
        let mux = Multiplexer::new();

        let (id, mut rx) = mux.register();
        mux.cancel(id);

        // Receiver should get cancellation error
        let result = rx.try_recv().unwrap();
        assert!(matches!(result, Err(NimbusError::Cancelled)));
    }

    #[test]
    fn test_cancel_all() {
        let mux = Multiplexer::new();

        let (_id1, rx1) = mux.register();
        let (_id2, rx2) = mux.register();
        let (_id3, rx3) = mux.register();

        assert_eq!(mux.pending_count(), 3);

        mux.cancel_all();

        assert_eq!(mux.pending_count(), 0);

        // All receivers should get connection closed error
        for mut rx in [rx1, rx2, rx3] {
            let result = rx.try_recv().unwrap();
            assert!(matches!(
                result,
                Err(NimbusError::Transport(TransportError::ConnectionClosed))
            ));
        }
    }

    #[test]
    fn test_unique_ids() {
        let mux = Multiplexer::new();

        let (id1, _) = mux.register();
        let (id2, _) = mux.register();
        let (id3, _) = mux.register();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }
}
