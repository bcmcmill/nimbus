//! Connection pool benchmarks.
//!
//! These benchmarks measure connection pool performance
//! including acquisition, return, and concurrent access patterns.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use nimbus_transport::{ConnectionPool, PoolConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

/// Create a pool for benchmarking.
fn create_pool() -> ConnectionPool<String> {
    let config = PoolConfig::default()
        .max_connections(64)
        .idle_timeout(Duration::from_secs(300));
    ConnectionPool::new(config)
}

/// Benchmark pool miss (creating new connection).
fn bench_pool_miss(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("pool_miss");

    group.bench_function("create_new", |b| {
        let pool = create_pool();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        b.to_async(&rt).iter(|| async {
            // Each iteration creates a new connection (pool miss)
            let conn = pool
                .get_or_create(addr, || async { Ok("connection".to_string()) })
                .await
                .unwrap();
            // Take ownership to prevent return to pool
            black_box(conn.take());
        });
    });

    group.finish();
}

/// Benchmark pool hit (reusing existing connection).
fn bench_pool_hit(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("pool_hit");

    group.bench_function("reuse_existing", |b| {
        let pool = create_pool();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        // Warm up - create and return a connection
        rt.block_on(async {
            let conn = pool
                .get_or_create(addr, || async { Ok("connection".to_string()) })
                .await
                .unwrap();
            drop(conn); // Return to pool
        });

        b.to_async(&rt).iter(|| async {
            // Should hit the pool
            let conn = pool
                .get_or_create(addr, || async { Ok("new_connection".to_string()) })
                .await
                .unwrap();
            black_box(&conn);
            // Connection automatically returned on drop
        });
    });

    group.finish();
}

/// Benchmark concurrent pool access.
fn bench_pool_concurrent(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("pool_concurrent");

    for concurrency in [4, 16, 64] {
        group.throughput(Throughput::Elements(concurrency as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            &concurrency,
            |b, &concurrency| {
                let pool = Arc::new(create_pool());
                let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

                // Pre-warm pool
                rt.block_on(async {
                    for _ in 0..concurrency {
                        let conn = pool
                            .get_or_create(addr, || async { Ok("conn".to_string()) })
                            .await
                            .unwrap();
                        drop(conn);
                    }
                });

                b.to_async(&rt).iter(|| {
                    let pool = pool.clone();
                    async move {
                        let handles: Vec<_> = (0..concurrency)
                            .map(|_| {
                                let pool = pool.clone();
                                tokio::spawn(async move {
                                    let conn = pool
                                        .get_or_create(addr, || async { Ok("conn".to_string()) })
                                        .await
                                        .unwrap();
                                    black_box(&conn);
                                })
                            })
                            .collect();

                        for handle in handles {
                            handle.await.unwrap();
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark connection return overhead.
fn bench_pool_return(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("pool_return");

    group.bench_function("acquire_and_return", |b| {
        let pool = create_pool();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        // Pre-warm
        rt.block_on(async {
            let conn = pool
                .get_or_create(addr, || async { Ok("conn".to_string()) })
                .await
                .unwrap();
            drop(conn);
        });

        b.to_async(&rt).iter(|| async {
            let conn = pool
                .get_or_create(addr, || async { Ok("conn".to_string()) })
                .await
                .unwrap();
            drop(black_box(conn)); // Measure return time too
        });
    });

    group.finish();
}

/// Benchmark pool stats lookup.
fn bench_pool_stats(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool_stats");
    let rt = Runtime::new().unwrap();

    let pool = create_pool();
    let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

    // Create some connections
    rt.block_on(async {
        for _ in 0..8 {
            let conn = pool
                .get_or_create(addr, || async { Ok("conn".to_string()) })
                .await
                .unwrap();
            drop(conn);
        }
    });

    group.bench_function("stats_lookup", |b| {
        b.iter(|| {
            let stats = pool.stats(black_box(&addr));
            black_box(stats);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_pool_miss,
    bench_pool_hit,
    bench_pool_concurrent,
    bench_pool_return,
    bench_pool_stats,
);

criterion_main!(benches);
