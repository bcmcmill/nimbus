//! DNS-based service discovery.

use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::channel::mpsc;
use futures_core::Stream;
use hickory_resolver::Resolver as HickoryResolver;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;

use crate::resolver::{Endpoint, ResolveError, Resolver};

/// Type alias for the Tokio-based hickory resolver.
type TokioResolver = HickoryResolver<TokioConnectionProvider>;

/// DNS-based service discovery resolver.
///
/// Supports:
/// - SRV record lookup (recommended for service discovery)
/// - A/AAAA record lookup with default port
///
/// ## Example
///
/// ```rust
/// use nimbus_discovery::DnsResolver;
///
/// // Create a DNS resolver for service discovery
/// let resolver = DnsResolver::new();
///
/// // Create a resolver with a default port for A/AAAA lookups
/// let resolver_with_port = DnsResolver::with_default_port(8080);
///
/// // Actual DNS resolution requires network access and is async:
/// // let endpoints = resolver.resolve("_myservice._tcp.example.com").await?;
/// ```
pub struct DnsResolver {
    resolver: TokioResolver,
    default_port: Option<u16>,
}

impl DnsResolver {
    /// Create a new DNS resolver with system configuration.
    #[must_use]
    pub fn new() -> Self {
        let resolver = HickoryResolver::builder_with_config(
            ResolverConfig::default(),
            TokioConnectionProvider::default(),
        )
        .with_options(ResolverOpts::default())
        .build();

        Self {
            resolver,
            default_port: None,
        }
    }

    /// Create a resolver with a default port for A/AAAA lookups.
    #[must_use]
    pub fn with_default_port(port: u16) -> Self {
        let mut resolver = Self::new();
        resolver.default_port = Some(port);
        resolver
    }

    /// Create a resolver with custom configuration.
    pub fn with_config(config: ResolverConfig, opts: ResolverOpts) -> Self {
        let resolver =
            HickoryResolver::builder_with_config(config, TokioConnectionProvider::default())
                .with_options(opts)
                .build();

        Self {
            resolver,
            default_port: None,
        }
    }

    /// Resolve SRV records for a service.
    async fn resolve_srv(&self, name: &str) -> Result<Vec<Endpoint>, ResolveError> {
        let lookup = self
            .resolver
            .srv_lookup(name)
            .await
            .map_err(|e| ResolveError::Dns(e.to_string()))?;

        let mut endpoints = Vec::new();

        for record in lookup.iter() {
            // Resolve the target hostname to IP addresses
            let target = record.target().to_string();
            let port = record.port();
            let priority = record.priority() as u32;
            let weight = record.weight() as u32;

            // Look up A/AAAA records for the target
            if let Ok(ips) = self.resolver.lookup_ip(&target).await {
                for ip in ips.iter() {
                    endpoints.push(
                        Endpoint::new(SocketAddr::new(ip, port))
                            .with_priority(priority)
                            .with_weight(weight.max(1)), // Ensure weight is at least 1
                    );
                }
            }
        }

        if endpoints.is_empty() {
            return Err(ResolveError::NotFound(name.to_string()));
        }

        // Sort by priority (lower is better), then by weight
        endpoints.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| b.weight.cmp(&a.weight))
        });

        Ok(endpoints)
    }

    /// Resolve A/AAAA records for a hostname.
    async fn resolve_host(&self, name: &str) -> Result<Vec<Endpoint>, ResolveError> {
        let port = self.default_port.ok_or_else(|| {
            ResolveError::Dns("No port specified and no default port configured".to_string())
        })?;

        let lookup = self
            .resolver
            .lookup_ip(name)
            .await
            .map_err(|e| ResolveError::Dns(e.to_string()))?;

        let endpoints: Vec<Endpoint> = lookup
            .iter()
            .map(|ip| Endpoint::new(SocketAddr::new(ip, port)))
            .collect();

        if endpoints.is_empty() {
            return Err(ResolveError::NotFound(name.to_string()));
        }

        Ok(endpoints)
    }
}

impl Default for DnsResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Resolver for DnsResolver {
    async fn resolve(&self, service: &str) -> Result<Vec<Endpoint>, ResolveError> {
        // Try SRV lookup first (if name looks like an SRV record)
        if service.starts_with('_') {
            return self.resolve_srv(service).await;
        }

        // Try A/AAAA lookup if default port is set
        if self.default_port.is_some() {
            return self.resolve_host(service).await;
        }

        // Try SRV anyway
        self.resolve_srv(service).await
    }

    fn watch(&self, service: &str) -> impl Stream<Item = Vec<Endpoint>> + Send + Unpin {
        let resolver = Self::new();
        let service = service.to_string();

        // Create an mpsc channel for updates
        let (mut tx, rx) = mpsc::channel(16);

        // Spawn a task to periodically resolve and update
        ntex::rt::spawn(async move {
            let interval = ntex::time::interval(std::time::Duration::from_secs(30));

            loop {
                interval.tick().await;

                if let Ok(endpoints) = resolver.resolve(&service).await {
                    if tx.try_send(endpoints).is_err() {
                        break; // Receiver dropped or buffer full
                    }
                }
            }
        });

        // Wrap the receiver as a Stream
        EndpointStream { rx }
    }
}

/// Stream wrapper for endpoint updates from DNS resolution.
struct EndpointStream {
    rx: mpsc::Receiver<Vec<Endpoint>>,
}

impl Stream for EndpointStream {
    type Item = Vec<Endpoint>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx).poll_next(cx)
    }
}

impl Unpin for EndpointStream {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_resolver_creation() {
        let resolver = DnsResolver::new();
        assert!(resolver.default_port.is_none());

        let resolver = DnsResolver::with_default_port(8080);
        assert_eq!(resolver.default_port, Some(8080));
    }

    // Note: Actual DNS resolution tests would require network access
    // or a mock DNS server, so they're typically integration tests.
}
