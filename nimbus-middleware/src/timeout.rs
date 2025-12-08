//! Timeout interceptor for enforcing request deadlines.

use std::time::Duration;

use nimbus_core::Context;

use crate::interceptor::{Interceptor, InterceptorError};

/// Interceptor that enforces request timeouts.
///
/// This interceptor ensures that all requests have a deadline set.
/// If a request already has a deadline, it will not be modified
/// (unless the existing deadline exceeds the maximum allowed).
///
/// ## Example
///
/// ```rust
/// use nimbus_middleware::TimeoutInterceptor;
/// use std::time::Duration;
///
/// let interceptor = TimeoutInterceptor::new(Duration::from_secs(30));
/// ```
pub struct TimeoutInterceptor {
    /// Default timeout for requests without a deadline.
    default_timeout: Duration,
    /// Maximum allowed timeout (enforced even if request has longer deadline).
    max_timeout: Option<Duration>,
}

impl TimeoutInterceptor {
    /// Create a new timeout interceptor with the given default timeout.
    #[must_use]
    pub fn new(default_timeout: Duration) -> Self {
        Self {
            default_timeout,
            max_timeout: None,
        }
    }

    /// Set a maximum timeout that will be enforced even if the request
    /// has a longer deadline.
    #[must_use]
    pub fn with_max(mut self, max_timeout: Duration) -> Self {
        self.max_timeout = Some(max_timeout);
        self
    }
}

impl Interceptor for TimeoutInterceptor {
    fn intercept_request(
        &self,
        ctx: &mut Context,
        _request: &[u8],
    ) -> Result<(), InterceptorError> {
        let now = std::time::Instant::now();

        match ctx.deadline {
            Some(deadline) => {
                // Check if existing deadline exceeds max
                if let Some(max) = self.max_timeout {
                    let max_deadline = now + max;
                    if deadline > max_deadline {
                        ctx.deadline = Some(max_deadline);
                    }
                }
            }
            None => {
                // Set default deadline
                ctx.deadline = Some(now + self.default_timeout);
            }
        }

        // Check if already expired
        if ctx.is_expired() {
            return Err(InterceptorError::Rejected("request already expired".into()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_sets_default_deadline() {
        let interceptor = TimeoutInterceptor::new(Duration::from_secs(30));
        let mut ctx = Context::new();

        interceptor.intercept_request(&mut ctx, &[]).unwrap();

        assert!(ctx.deadline.is_some());
        assert!(ctx.remaining().unwrap() <= Duration::from_secs(30));
    }

    #[test]
    fn test_preserves_existing_deadline() {
        let interceptor = TimeoutInterceptor::new(Duration::from_secs(30));
        let mut ctx = Context::new();

        let original_deadline = Instant::now() + Duration::from_secs(10);
        ctx.deadline = Some(original_deadline);

        interceptor.intercept_request(&mut ctx, &[]).unwrap();

        assert_eq!(ctx.deadline, Some(original_deadline));
    }

    #[test]
    fn test_enforces_max_deadline() {
        let interceptor = TimeoutInterceptor::new(Duration::from_secs(30))
            .with_max(Duration::from_secs(5));

        let mut ctx = Context::new();
        ctx.deadline = Some(Instant::now() + Duration::from_secs(100));

        interceptor.intercept_request(&mut ctx, &[]).unwrap();

        // Should be reduced to max
        assert!(ctx.remaining().unwrap() <= Duration::from_secs(5));
    }

    #[test]
    fn test_rejects_expired() {
        let interceptor = TimeoutInterceptor::new(Duration::from_secs(30));
        let mut ctx = Context::new();

        // Set deadline in the past
        ctx.deadline = Some(Instant::now() - Duration::from_secs(1));

        let result = interceptor.intercept_request(&mut ctx, &[]);
        assert!(result.is_err());
    }
}
