//! Connection pooling for RPC clients.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_lock::Semaphore;
use dashmap::DashMap;
use futures::future::{Either, select};
use futures_timer::Delay;
use parking_lot::Mutex;
use std::pin::pin;

use nimbus_core::TransportError;

/// Configuration for connection pooling.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum connections per endpoint.
    pub max_connections_per_endpoint: usize,

    /// Minimum idle connections per endpoint.
    pub min_idle_connections: usize,

    /// Maximum lifetime of a connection.
    pub max_lifetime: Duration,

    /// Idle timeout before closing a connection.
    pub idle_timeout: Duration,

    /// Timeout for acquiring a connection from the pool.
    pub acquire_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections_per_endpoint: 16,
            min_idle_connections: 2,
            max_lifetime: Duration::from_secs(300),
            idle_timeout: Duration::from_secs(60),
            acquire_timeout: Duration::from_secs(5),
        }
    }
}

impl PoolConfig {
    /// Create a new pool configuration with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum connections per endpoint.
    #[must_use]
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections_per_endpoint = max;
        self
    }

    /// Set minimum idle connections.
    #[must_use]
    pub fn min_idle(mut self, min: usize) -> Self {
        self.min_idle_connections = min;
        self
    }

    /// Set maximum connection lifetime.
    #[must_use]
    pub fn max_lifetime(mut self, lifetime: Duration) -> Self {
        self.max_lifetime = lifetime;
        self
    }

    /// Set idle timeout.
    #[must_use]
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }
}

/// A pooled connection wrapper.
///
/// Note: This pool requires connections to be `Send` because it's designed
/// for multi-threaded use. For single-threaded ntex workers, use direct
/// connection management instead of pooling.
pub struct PooledConnection<C: Send + 'static> {
    connection: Option<C>,
    addr: SocketAddr,
    created: Instant,
    last_used: Instant,
    pool: Option<Arc<ConnectionPoolInner<C>>>,
}

impl<C: Send + 'static> PooledConnection<C> {
    /// Get the connection.
    pub fn connection(&self) -> &C {
        self.connection.as_ref().expect("connection taken")
    }

    /// Get mutable access to the connection.
    pub fn connection_mut(&mut self) -> &mut C {
        self.connection.as_mut().expect("connection taken")
    }

    /// Take ownership of the connection (removing it from the pool).
    pub fn take(mut self) -> C {
        // Release the slot back to the pool before taking the connection
        if let Some(pool) = self.pool.take() {
            pool.release_slot(self.addr);
        }
        self.connection.take().expect("connection already taken")
    }

    /// Check if the connection has expired.
    pub fn is_expired(&self, max_lifetime: Duration) -> bool {
        self.created.elapsed() > max_lifetime
    }

    /// Check if the connection has been idle too long.
    pub fn is_idle(&self, idle_timeout: Duration) -> bool {
        self.last_used.elapsed() > idle_timeout
    }
}

impl<C: Send + 'static> Drop for PooledConnection<C> {
    fn drop(&mut self) {
        if let (Some(pool), Some(conn)) = (self.pool.take(), self.connection.take()) {
            pool.release(conn, self.addr, self.created);
        }
    }
}

struct EndpointPool<C> {
    connections: Mutex<Vec<IdleConnection<C>>>,
    semaphore: Semaphore,
}

struct IdleConnection<C> {
    connection: C,
    created: Instant,
    returned: Instant,
}

struct ConnectionPoolInner<C> {
    endpoints: DashMap<SocketAddr, EndpointPool<C>>,
    config: PoolConfig,
    closed: AtomicBool,
}

impl<C: Send + 'static> ConnectionPoolInner<C> {
    fn release(&self, connection: C, addr: SocketAddr, created: Instant) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }

        // Don't return expired connections
        if created.elapsed() > self.config.max_lifetime {
            // Still need to release the slot even if connection is expired
            self.release_slot(addr);
            return;
        }

        if let Some(endpoint) = self.endpoints.get(&addr) {
            let mut conns = endpoint.connections.lock();
            conns.push(IdleConnection {
                connection,
                created,
                returned: Instant::now(),
            });
            endpoint.semaphore.add_permits(1);
        }
    }

    /// Release a connection slot without returning the connection to the pool.
    /// Called when a connection is permanently taken out of the pool.
    fn release_slot(&self, addr: SocketAddr) {
        if let Some(endpoint) = self.endpoints.get(&addr) {
            endpoint.semaphore.add_permits(1);
        }
    }

    fn cleanup(&self) {
        for entry in self.endpoints.iter_mut() {
            let mut conns = entry.value().connections.lock();
            let before = conns.len();
            conns.retain(|c| {
                c.created.elapsed() <= self.config.max_lifetime
                    && c.returned.elapsed() <= self.config.idle_timeout
            });
            let removed = before - conns.len();
            if removed > 0 {
                // Don't add permits back - they represent removed connections
            }
        }
    }
}

/// Connection pool for managing connections to multiple endpoints.
///
/// The pool maintains a set of connections per endpoint, automatically
/// managing lifecycle, idle timeouts, and connection limits.
///
/// ## Example
///
/// ```rust
/// use nimbus_transport::{ConnectionPool, PoolConfig};
/// use std::time::Duration;
///
/// // Configure the pool
/// let config = PoolConfig::default()
///     .max_connections(32)
///     .idle_timeout(Duration::from_secs(120));
///
/// // Create a pool (generic over connection type)
/// let pool: ConnectionPool<String> = ConnectionPool::new(config);
///
/// // Use get_or_create to obtain connections:
/// // let conn = pool.get_or_create(addr, || async { create_connection() }).await?;
/// ```
pub struct ConnectionPool<C> {
    inner: Arc<ConnectionPoolInner<C>>,
}

impl<C: Send + 'static> ConnectionPool<C> {
    /// Create a new connection pool.
    #[must_use]
    pub fn new(config: PoolConfig) -> Self {
        Self {
            inner: Arc::new(ConnectionPoolInner {
                endpoints: DashMap::new(),
                config,
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// Get a connection from the pool, or create one if none available.
    pub async fn get_or_create<F, Fut>(
        &self,
        addr: SocketAddr,
        create: F,
    ) -> Result<PooledConnection<C>, TransportError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<C, TransportError>>,
    {
        if self.inner.closed.load(Ordering::Relaxed) {
            return Err(TransportError::ConnectionClosed);
        }

        // Ensure endpoint pool exists
        self.inner
            .endpoints
            .entry(addr)
            .or_insert_with(|| EndpointPool {
                connections: Mutex::new(Vec::new()),
                semaphore: Semaphore::new(self.inner.config.max_connections_per_endpoint),
            });

        let endpoint = self.inner.endpoints.get(&addr).unwrap();

        // Try to get an idle connection
        {
            let mut conns = endpoint.connections.lock();
            while let Some(idle) = conns.pop() {
                // Check if connection is still valid
                if idle.created.elapsed() <= self.inner.config.max_lifetime
                    && idle.returned.elapsed() <= self.inner.config.idle_timeout
                {
                    return Ok(PooledConnection {
                        connection: Some(idle.connection),
                        addr,
                        created: idle.created,
                        last_used: Instant::now(),
                        pool: Some(self.inner.clone()),
                    });
                }
                // Connection expired, don't return permit (connection is gone)
            }
        }

        // Need to create a new connection
        // Try to acquire a permit (respects max connections)
        let acquire_fut = pin!(endpoint.semaphore.acquire());
        let timeout_fut = pin!(Delay::new(self.inner.config.acquire_timeout));

        let permit = match select(acquire_fut, timeout_fut).await {
            Either::Left((permit, _)) => permit,
            Either::Right(_) => return Err(TransportError::PoolExhausted),
        };

        // Create the connection
        let connection = create().await?;

        // Forget the permit - it will be returned when the connection is returned to pool
        std::mem::forget(permit);

        Ok(PooledConnection {
            connection: Some(connection),
            addr,
            created: Instant::now(),
            last_used: Instant::now(),
            pool: Some(self.inner.clone()),
        })
    }

    /// Get statistics for an endpoint.
    #[must_use]
    pub fn stats(&self, addr: &SocketAddr) -> Option<PoolStats> {
        self.inner.endpoints.get(addr).map(|e| {
            let idle = e.connections.lock().len();
            PoolStats {
                idle_connections: idle,
                max_connections: self.inner.config.max_connections_per_endpoint,
            }
        })
    }

    /// Run cleanup to remove expired connections.
    pub fn cleanup(&self) {
        self.inner.cleanup();
    }

    /// Close the pool, preventing new connections.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::Relaxed);
        self.inner.endpoints.clear();
    }
}

impl<C: Send + 'static> Clone for ConnectionPool<C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Statistics for a pool endpoint.
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Number of idle connections.
    pub idle_connections: usize,
    /// Maximum connections allowed.
    pub max_connections: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[ntex::test]
    async fn test_pool_create() {
        let pool: ConnectionPool<String> = ConnectionPool::new(PoolConfig::default());

        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let conn = pool
            .get_or_create(addr, || async { Ok("test connection".to_string()) })
            .await
            .unwrap();

        assert_eq!(conn.connection(), "test connection");
    }

    #[ntex::test]
    async fn test_pool_reuse() {
        let pool: ConnectionPool<usize> = ConnectionPool::new(PoolConfig::default());
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        let mut counter = 0;

        // First connection
        let conn1 = pool
            .get_or_create(addr, || {
                counter += 1;
                let c = counter;
                async move { Ok(c) }
            })
            .await
            .unwrap();
        let id1 = *conn1.connection();
        drop(conn1);

        // Second connection should reuse
        let conn2 = pool
            .get_or_create(addr, || {
                counter += 1;
                let c = counter;
                async move { Ok(c) }
            })
            .await
            .unwrap();
        let id2 = *conn2.connection();

        assert_eq!(id1, id2); // Should be same connection
    }
}
