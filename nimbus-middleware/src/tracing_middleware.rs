//! Distributed tracing interceptor.

use nimbus_core::{Context, NimbusError, TraceId};
use tracing::{Instrument, Span, info_span};

use crate::interceptor::{Interceptor, InterceptorError};

/// Interceptor for distributed tracing integration.
///
/// This interceptor:
/// - Ensures each request has a trace ID
/// - Creates tracing spans for requests
/// - Propagates trace context in metadata
///
/// ## Example
///
/// ```rust,ignore
/// use nimbus_middleware::TracingInterceptor;
///
/// let interceptor = TracingInterceptor::new("my-service");
/// ```
pub struct TracingInterceptor {
    service_name: String,
}

impl TracingInterceptor {
    /// Create a new tracing interceptor.
    #[must_use]
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
        }
    }
}

impl Interceptor for TracingInterceptor {
    fn intercept_request(
        &self,
        ctx: &mut Context,
        _request: &[u8],
    ) -> Result<(), InterceptorError> {
        // Ensure trace ID exists
        if ctx.trace_id.is_none() {
            ctx.trace_id = Some(TraceId::new());
        }

        // Add trace ID to metadata for propagation
        if let Some(ref trace_id) = ctx.trace_id {
            ctx.metadata
                .insert("x-trace-id".to_string(), trace_id.to_string());
        }

        // Add request ID
        ctx.metadata
            .insert("x-request-id".to_string(), ctx.request_id.to_string());

        // Add service name
        ctx.metadata
            .insert("x-service".to_string(), self.service_name.clone());

        Ok(())
    }

    fn on_error(&self, ctx: &Context, error: &NimbusError) {
        let trace_id = ctx
            .trace_id
            .as_ref()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        tracing::error!(
            trace_id = %trace_id,
            request_id = ctx.request_id,
            error = %error,
            "RPC request failed"
        );
    }
}

/// Extension trait for creating spans from context.
pub trait ContextSpanExt {
    /// Create a tracing span for this context.
    fn span(&self, operation: &str) -> Span;
}

impl ContextSpanExt for Context {
    fn span(&self, operation: &str) -> Span {
        let trace_id = self
            .trace_id
            .as_ref()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        info_span!(
            "rpc",
            operation = operation,
            trace_id = %trace_id,
            request_id = self.request_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adds_trace_id() {
        let interceptor = TracingInterceptor::new("test-service");
        let mut ctx = Context::new();

        interceptor.intercept_request(&mut ctx, &[]).unwrap();

        assert!(ctx.trace_id.is_some());
        assert!(ctx.metadata.contains_key("x-trace-id"));
    }

    #[test]
    fn test_preserves_existing_trace_id() {
        let interceptor = TracingInterceptor::new("test-service");
        let mut ctx = Context::new();

        let original_trace_id = TraceId::new();
        ctx.trace_id = Some(original_trace_id.clone());

        interceptor.intercept_request(&mut ctx, &[]).unwrap();

        assert_eq!(
            ctx.trace_id.as_ref().unwrap().as_bytes(),
            original_trace_id.as_bytes()
        );
    }
}
