//! # nimbus-middleware
//!
//! Middleware and interceptors for the Nimbus RPC framework.
//!
//! This crate provides:
//! - `Interceptor` trait for request/response interception
//! - `TimeoutInterceptor` for enforcing deadlines
//! - `RetryInterceptor` for automatic retries with backoff
//! - `TracingInterceptor` for distributed tracing (with `tracing` feature)

mod interceptor;
mod retry;
mod timeout;

#[cfg(feature = "tracing")]
mod tracing_middleware;

pub use interceptor::{Interceptor, InterceptorChain, InterceptorError};
pub use retry::{RetryConfig, RetryInterceptor};
pub use timeout::TimeoutInterceptor;

#[cfg(feature = "tracing")]
pub use tracing_middleware::TracingInterceptor;
