//! Resolver trait and common types.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use futures_core::Stream;

/// Error type for resolution operations.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// Service not found.
    #[error("service not found: {0}")]
    NotFound(String),

    /// DNS resolution failed.
    #[error("dns error: {0}")]
    Dns(String),

    /// Registry communication failed.
    #[error("registry error: {0}")]
    Registry(String),

    /// Resolution timed out.
    #[error("resolution timed out")]
    Timeout,

    /// No healthy endpoints available.
    #[error("no healthy endpoints")]
    NoHealthyEndpoints,
}

/// A service endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Socket address of the endpoint.
    pub addr: SocketAddr,

    /// Weight for load balancing (higher = more traffic).
    pub weight: u32,

    /// Priority (lower = preferred).
    pub priority: u32,

    /// Optional metadata.
    pub metadata: Option<Arc<std::collections::HashMap<String, String>>>,
}

impl Endpoint {
    /// Create a new endpoint with default weight and priority.
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            weight: 100,
            priority: 0,
            metadata: None,
        }
    }

    /// Create an endpoint with weight.
    #[must_use]
    pub fn with_weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }

    /// Create an endpoint with priority.
    #[must_use]
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }
}

impl From<SocketAddr> for Endpoint {
    fn from(addr: SocketAddr) -> Self {
        Self::new(addr)
    }
}

/// Trait for service discovery resolvers.
///
/// Implementations provide methods to resolve service names to endpoints
/// and optionally watch for changes.
///
/// ## Example
///
/// ```rust
/// use nimbus_discovery::{Resolver, StaticResolver, Endpoint};
/// use std::net::SocketAddr;
///
/// // Create a static resolver for testing
/// let resolver = StaticResolver::new();
/// let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
/// resolver.add_endpoint("my-service", addr);
///
/// // Resolution is async - here's how you'd use it:
/// // let endpoints = resolver.resolve("my-service").await?;
/// ```
pub trait Resolver: Send + Sync + 'static {
    /// Resolve a service name to a list of endpoints.
    fn resolve(
        &self,
        service: &str,
    ) -> impl Future<Output = Result<Vec<Endpoint>, ResolveError>> + Send;

    /// Watch for changes to a service's endpoints.
    ///
    /// Returns a stream that yields updated endpoint lists whenever
    /// the service configuration changes.
    fn watch(
        &self,
        service: &str,
    ) -> impl Stream<Item = Vec<Endpoint>> + Send + Unpin;
}

/// A static resolver that returns pre-configured endpoints.
///
/// Useful for testing or when endpoints are known at configuration time.
pub struct StaticResolver {
    endpoints: DashMap<String, Vec<Endpoint>>,
}

impl StaticResolver {
    /// Create a new empty static resolver.
    #[must_use]
    pub fn new() -> Self {
        Self {
            endpoints: DashMap::new(),
        }
    }

    /// Add endpoints for a service.
    pub fn add_service(&self, service: impl Into<String>, endpoints: Vec<Endpoint>) {
        self.endpoints.insert(service.into(), endpoints);
    }

    /// Add a single endpoint for a service.
    pub fn add_endpoint(&self, service: impl Into<String>, endpoint: impl Into<Endpoint>) {
        self.endpoints
            .entry(service.into())
            .or_insert_with(Vec::new)
            .push(endpoint.into());
    }
}

impl Default for StaticResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Resolver for StaticResolver {
    async fn resolve(&self, service: &str) -> Result<Vec<Endpoint>, ResolveError> {
        self.endpoints
            .get(service)
            .map(|e| e.clone())
            .ok_or_else(|| ResolveError::NotFound(service.to_string()))
    }

    fn watch(&self, service: &str) -> impl Stream<Item = Vec<Endpoint>> + Send + Unpin {
        let endpoints = self
            .endpoints
            .get(service)
            .map(|e| e.clone())
            .unwrap_or_default();

        // Static resolver just returns the initial value once
        Box::pin(futures::stream::once(async move { endpoints }))
    }
}

/// Caching wrapper around a resolver.
pub struct CachingResolver<R> {
    inner: R,
    cache: DashMap<String, CacheEntry>,
    ttl: Duration,
}

struct CacheEntry {
    endpoints: Vec<Endpoint>,
    expires: std::time::Instant,
}

impl<R: Resolver> CachingResolver<R> {
    /// Create a new caching resolver.
    #[must_use]
    pub fn new(inner: R, ttl: Duration) -> Self {
        Self {
            inner,
            cache: DashMap::new(),
            ttl,
        }
    }

    /// Clear the cache.
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// Invalidate a specific service's cache entry.
    pub fn invalidate(&self, service: &str) {
        self.cache.remove(service);
    }
}

impl<R: Resolver> Resolver for CachingResolver<R> {
    async fn resolve(&self, service: &str) -> Result<Vec<Endpoint>, ResolveError> {
        let now = std::time::Instant::now();

        // Check cache
        if let Some(entry) = self.cache.get(service) {
            if entry.expires > now {
                return Ok(entry.endpoints.clone());
            }
        }

        // Resolve and cache
        let endpoints = self.inner.resolve(service).await?;

        self.cache.insert(
            service.to_string(),
            CacheEntry {
                endpoints: endpoints.clone(),
                expires: now + self.ttl,
            },
        );

        Ok(endpoints)
    }

    fn watch(&self, service: &str) -> impl Stream<Item = Vec<Endpoint>> + Send + Unpin {
        // Delegate to inner resolver for watching
        self.inner.watch(service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_static_resolver() {
        let resolver = StaticResolver::new();
        resolver.add_endpoint("my-service", "127.0.0.1:8080".parse::<SocketAddr>().unwrap());
        resolver.add_endpoint("my-service", "127.0.0.1:8081".parse::<SocketAddr>().unwrap());

        let endpoints = resolver.resolve("my-service").await.unwrap();
        assert_eq!(endpoints.len(), 2);
    }

    #[tokio::test]
    async fn test_static_resolver_not_found() {
        let resolver = StaticResolver::new();
        let result = resolver.resolve("unknown").await;
        assert!(matches!(result, Err(ResolveError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_caching_resolver() {
        let inner = StaticResolver::new();
        inner.add_endpoint("test", "127.0.0.1:8080".parse::<SocketAddr>().unwrap());

        let resolver = CachingResolver::new(inner, Duration::from_secs(60));

        // First resolve
        let e1 = resolver.resolve("test").await.unwrap();

        // Second resolve (should use cache)
        let e2 = resolver.resolve("test").await.unwrap();

        assert_eq!(e1, e2);
    }
}
