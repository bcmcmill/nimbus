//! Interceptor trait and chain implementation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nimbus_core::{Context, NimbusError};

/// Error type for interceptor operations.
#[derive(Debug, thiserror::Error)]
pub enum InterceptorError {
    /// Request was rejected by the interceptor.
    #[error("request rejected: {0}")]
    Rejected(String),

    /// Interceptor internal error.
    #[error("interceptor error: {0}")]
    Internal(String),
}

impl From<InterceptorError> for NimbusError {
    fn from(err: InterceptorError) -> Self {
        NimbusError::Service {
            code: 1,
            message: err.to_string(),
        }
    }
}

/// Trait for intercepting RPC requests and responses.
///
/// Interceptors can modify requests before they are sent, and responses
/// after they are received. They can also reject requests entirely.
///
/// ## Example
///
/// ```rust
/// use nimbus_middleware::{Interceptor, InterceptorError};
/// use nimbus_core::Context;
///
/// struct AuthInterceptor {
///     api_key: String,
/// }
///
/// impl Interceptor for AuthInterceptor {
///     fn intercept_request(
///         &self,
///         ctx: &mut Context,
///         _request: &[u8],
///     ) -> Result<(), InterceptorError> {
///         ctx.metadata.insert("authorization".to_string(), self.api_key.clone());
///         Ok(())
///     }
/// }
/// ```
pub trait Interceptor: Send + Sync + 'static {
    /// Intercept an outgoing request.
    ///
    /// This is called before the request is serialized and sent.
    /// The interceptor can modify the context (e.g., add metadata)
    /// or reject the request by returning an error.
    fn intercept_request(&self, ctx: &mut Context, request: &[u8]) -> Result<(), InterceptorError> {
        let _ = (ctx, request);
        Ok(())
    }

    /// Intercept an incoming response.
    ///
    /// This is called after the response is received but before
    /// it is deserialized and returned to the caller.
    fn intercept_response(
        &self,
        ctx: &Context,
        response: &mut Vec<u8>,
    ) -> Result<(), InterceptorError> {
        let _ = (ctx, response);
        Ok(())
    }

    /// Called when a request fails.
    ///
    /// This can be used for logging or metrics collection.
    fn on_error(&self, ctx: &Context, error: &NimbusError) {
        let _ = (ctx, error);
    }
}

/// A chain of interceptors.
///
/// Interceptors are executed in order for requests (first to last)
/// and in reverse order for responses (last to first).
pub struct InterceptorChain {
    interceptors: Vec<Arc<dyn Interceptor>>,
}

impl InterceptorChain {
    /// Create a new empty interceptor chain.
    #[must_use]
    pub fn new() -> Self {
        Self {
            interceptors: Vec::new(),
        }
    }

    /// Add an interceptor to the chain.
    #[must_use]
    pub fn with(mut self, interceptor: impl Interceptor) -> Self {
        self.interceptors.push(Arc::new(interceptor));
        self
    }

    /// Add a shared interceptor to the chain.
    #[must_use]
    pub fn with_shared(mut self, interceptor: Arc<dyn Interceptor>) -> Self {
        self.interceptors.push(interceptor);
        self
    }

    /// Process a request through all interceptors.
    pub fn intercept_request(
        &self,
        ctx: &mut Context,
        request: &[u8],
    ) -> Result<(), InterceptorError> {
        for interceptor in &self.interceptors {
            interceptor.intercept_request(ctx, request)?;
        }
        Ok(())
    }

    /// Process a response through all interceptors (in reverse order).
    pub fn intercept_response(
        &self,
        ctx: &Context,
        response: &mut Vec<u8>,
    ) -> Result<(), InterceptorError> {
        for interceptor in self.interceptors.iter().rev() {
            interceptor.intercept_response(ctx, response)?;
        }
        Ok(())
    }

    /// Notify all interceptors of an error.
    pub fn on_error(&self, ctx: &Context, error: &NimbusError) {
        for interceptor in &self.interceptors {
            interceptor.on_error(ctx, error);
        }
    }

    /// Check if the chain is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.interceptors.is_empty()
    }

    /// Get the number of interceptors in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.interceptors.len()
    }
}

impl Default for InterceptorChain {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for InterceptorChain {
    fn clone(&self) -> Self {
        Self {
            interceptors: self.interceptors.clone(),
        }
    }
}

/// Async interceptor trait for interceptors that need to perform async operations.
pub trait AsyncInterceptor: Send + Sync + 'static {
    /// Intercept an outgoing request asynchronously.
    fn intercept_request<'a>(
        &'a self,
        ctx: &'a mut Context,
        request: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), InterceptorError>> + Send + 'a>>;

    /// Intercept an incoming response asynchronously.
    fn intercept_response<'a>(
        &'a self,
        ctx: &'a Context,
        response: &'a mut Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<(), InterceptorError>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestInterceptor {
        prefix: String,
    }

    impl Interceptor for TestInterceptor {
        fn intercept_request(
            &self,
            ctx: &mut Context,
            _request: &[u8],
        ) -> Result<(), InterceptorError> {
            ctx.metadata.insert("test".to_string(), self.prefix.clone());
            Ok(())
        }
    }

    #[test]
    fn test_interceptor_chain() {
        let chain = InterceptorChain::new()
            .with(TestInterceptor {
                prefix: "first".to_string(),
            })
            .with(TestInterceptor {
                prefix: "second".to_string(),
            });

        let mut ctx = Context::new();
        chain.intercept_request(&mut ctx, &[]).unwrap();

        // Second interceptor overwrites first
        assert_eq!(ctx.get_metadata("test"), Some("second"));
    }

    struct RejectingInterceptor;

    impl Interceptor for RejectingInterceptor {
        fn intercept_request(
            &self,
            _ctx: &mut Context,
            _request: &[u8],
        ) -> Result<(), InterceptorError> {
            Err(InterceptorError::Rejected("not allowed".into()))
        }
    }

    #[test]
    fn test_interceptor_rejection() {
        let chain = InterceptorChain::new().with(RejectingInterceptor);

        let mut ctx = Context::new();
        let result = chain.intercept_request(&mut ctx, &[]);

        assert!(result.is_err());
    }
}
