//! HTTP-based service registry client.
//!
//! This module provides a client for HTTP-based service registries
//! like Consul, etcd, or custom registry services.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use dashmap::DashMap;
use futures_core::Stream;
use tokio::sync::watch;

use crate::resolver::{Endpoint, ResolveError, Resolver};

/// Configuration for the registry client.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    /// Base URL of the registry service.
    pub endpoint: String,

    /// Timeout for registry requests.
    pub timeout: Duration,

    /// Interval for refreshing service endpoints.
    pub refresh_interval: Duration,

    /// Optional authentication token.
    pub auth_token: Option<String>,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:8500".to_string(),
            timeout: Duration::from_secs(5),
            refresh_interval: Duration::from_secs(30),
            auth_token: None,
        }
    }
}

impl RegistryConfig {
    /// Create a new config with the given endpoint.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            ..Default::default()
        }
    }

    /// Set the request timeout.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the refresh interval.
    #[must_use]
    pub fn refresh_interval(mut self, interval: Duration) -> Self {
        self.refresh_interval = interval;
        self
    }

    /// Set an authentication token.
    #[must_use]
    pub fn auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }
}

/// Client for HTTP-based service registries.
///
/// This client can work with various registry backends like Consul, etcd,
/// or custom registry services that implement a simple HTTP API.
///
/// ## Example
///
/// ```rust,ignore
/// use nimbus_discovery::RegistryClient;
///
/// let client = RegistryClient::new("http://consul:8500");
/// let endpoints = client.resolve("my-service").await?;
/// ```
pub struct RegistryClient {
    config: RegistryConfig,
    /// Cache of resolved endpoints.
    cache: DashMap<String, Vec<Endpoint>>,
    /// Watch channels for each service.
    watchers: DashMap<String, watch::Sender<Vec<Endpoint>>>,
}

impl RegistryClient {
    /// Create a new registry client with the given endpoint.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self::with_config(RegistryConfig::new(endpoint))
    }

    /// Create a client with full configuration.
    #[must_use]
    pub fn with_config(config: RegistryConfig) -> Self {
        Self {
            config,
            cache: DashMap::new(),
            watchers: DashMap::new(),
        }
    }

    /// Get the registry endpoint URL.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.config.endpoint
    }

    /// Register a service with the registry.
    ///
    /// This registers the local service instance with the registry,
    /// making it discoverable by other services.
    pub async fn register(
        &self,
        service_name: &str,
        instance_id: &str,
        addr: SocketAddr,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), ResolveError> {
        // In a full implementation, this would make an HTTP request to register
        // For now, we'll add to the local cache
        tracing::info!(
            service = service_name,
            instance = instance_id,
            addr = %addr,
            "Registering service with registry"
        );

        let endpoint = if let Some(meta) = metadata {
            Endpoint::new(addr).with_metadata(meta)
        } else {
            Endpoint::new(addr)
        };

        self.cache
            .entry(service_name.to_string())
            .or_insert_with(Vec::new)
            .push(endpoint);

        Ok(())
    }

    /// Deregister a service instance.
    pub async fn deregister(
        &self,
        service_name: &str,
        instance_id: &str,
    ) -> Result<(), ResolveError> {
        tracing::info!(
            service = service_name,
            instance = instance_id,
            "Deregistering service from registry"
        );

        // In a full implementation, this would make an HTTP request
        // For now, we just log
        Ok(())
    }

    /// Refresh the endpoint cache for a service.
    async fn refresh(&self, service: &str) -> Result<Vec<Endpoint>, ResolveError> {
        // In a full implementation, this would make an HTTP request to the registry
        // For now, we return cached values or an error
        tracing::debug!(service, "Refreshing endpoints from registry");

        self.cache
            .get(service)
            .map(|e| e.clone())
            .ok_or_else(|| ResolveError::Registry(format!("service not found: {}", service)))
    }
}

impl Resolver for RegistryClient {
    async fn resolve(&self, service: &str) -> Result<Vec<Endpoint>, ResolveError> {
        // Check cache first
        if let Some(endpoints) = self.cache.get(service) {
            if !endpoints.is_empty() {
                return Ok(endpoints.clone());
            }
        }

        // Refresh from registry
        self.refresh(service).await
    }

    fn watch(&self, service: &str) -> impl Stream<Item = Vec<Endpoint>> + Send + Unpin {
        let service = service.to_string();

        // Get or create a watch channel for this service
        let rx = self
            .watchers
            .entry(service.clone())
            .or_insert_with(|| {
                let initial = self
                    .cache
                    .get(&service)
                    .map(|e| e.clone())
                    .unwrap_or_default();
                let (tx, _) = watch::channel(initial);
                tx
            })
            .subscribe();

        // Convert watch receiver to a stream
        tokio_stream::wrappers::WatchStream::new(rx)
    }
}

/// Extension trait for Endpoint to add metadata.
trait EndpointExt {
    fn with_metadata(self, metadata: HashMap<String, String>) -> Self;
}

impl EndpointExt for Endpoint {
    fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = Some(std::sync::Arc::new(metadata));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_registry_client_creation() {
        let client = RegistryClient::new("http://localhost:8500");
        assert_eq!(client.endpoint(), "http://localhost:8500");
    }

    #[tokio::test]
    async fn test_registry_config() {
        let config = RegistryConfig::new("http://consul:8500")
            .timeout(Duration::from_secs(10))
            .refresh_interval(Duration::from_secs(60))
            .auth_token("my-token");

        assert_eq!(config.endpoint, "http://consul:8500");
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert_eq!(config.refresh_interval, Duration::from_secs(60));
        assert_eq!(config.auth_token, Some("my-token".to_string()));
    }

    #[tokio::test]
    async fn test_register_and_resolve() {
        let client = RegistryClient::new("http://localhost:8500");

        client
            .register(
                "my-service",
                "instance-1",
                "127.0.0.1:8080".parse().unwrap(),
                None,
            )
            .await
            .unwrap();

        let endpoints = client.resolve("my-service").await.unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(
            endpoints[0].addr,
            "127.0.0.1:8080".parse::<SocketAddr>().unwrap()
        );
    }
}
