//! Multiplexer benchmarks.
//!
//! These benchmarks measure request multiplexing performance
//! including registration, dispatch, and concurrent operations.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nimbus_transport::Multiplexer;
use rkyv::util::AlignedVec;
use tokio::runtime::Runtime;

/// Benchmark register/dispatch pairs.
fn bench_mux_register_dispatch(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("mux_register_dispatch");

    group.bench_function("single_pair", |b| {
        let mux = Multiplexer::new();

        b.to_async(&rt).iter(|| async {
            let (id, rx) = mux.register();

            // Simulate response arriving
            let mut response = AlignedVec::new();
            response.extend_from_slice(b"response data");
            mux.dispatch(black_box(id), response);

            let result = rx.await.unwrap();
            let _ = black_box(result);
        });
    });

    group.finish();
}

/// Benchmark concurrent pending requests.
fn bench_mux_concurrent_pending(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("mux_concurrent_pending");

    for count in [10, 100, 1000] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.to_async(&rt).iter(|| async {
                let mux = Multiplexer::new();

                // Register all requests
                let receivers: Vec<_> = (0..count)
                    .map(|_| {
                        let (id, rx) = mux.register();
                        (id, rx)
                    })
                    .collect();

                // Dispatch all responses
                for (id, _) in &receivers {
                    let mut response = AlignedVec::new();
                    response.extend_from_slice(b"response");
                    mux.dispatch(black_box(*id), response);
                }

                // Collect all responses
                for (_, rx) in receivers {
                    let result = rx.await.unwrap();
                    let _ = black_box(result);
                }
            });
        });
    }

    group.finish();
}

/// Benchmark ID generation throughput.
fn bench_mux_id_generation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("mux_id_generation");

    group.throughput(Throughput::Elements(1));

    group.bench_function("register_only", |b| {
        let mux = Multiplexer::new();

        b.to_async(&rt).iter(|| async {
            let (id, _rx) = mux.register();
            black_box(id);
            // Note: receiver is dropped, simulating timeout
        });
    });

    group.finish();
}

/// Benchmark cancellation overhead.
fn bench_mux_cancel(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("mux_cancel");

    group.bench_function("cancel_single", |b| {
        let mux = Multiplexer::new();

        b.to_async(&rt).iter(|| async {
            let (id, rx) = mux.register();
            mux.cancel(black_box(id));

            // Verify cancellation
            let result = rx.await;
            let _ = black_box(result);
        });
    });

    for count in [10, 100, 1000] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("cancel_all", count),
            &count,
            |b, &count| {
                b.to_async(&rt).iter(|| async {
                    let mux = Multiplexer::new();

                    // Register many requests
                    let receivers: Vec<_> = (0..count).map(|_| mux.register().1).collect();

                    // Cancel all at once
                    mux.cancel_all();

                    // Verify all cancelled
                    for rx in receivers {
                        let result = rx.await;
                        let _ = black_box(result);
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark dispatch to unknown ID (miss case).
fn bench_mux_dispatch_miss(c: &mut Criterion) {
    let mut group = c.benchmark_group("mux_dispatch_miss");

    group.bench_function("unknown_id", |b| {
        let mux = Multiplexer::new();

        b.iter(|| {
            let mut response = AlignedVec::new();
            response.extend_from_slice(b"response");
            let dispatched = mux.dispatch(black_box(999999), response);
            black_box(dispatched);
        });
    });

    group.finish();
}

/// Benchmark pending count lookup.
fn bench_mux_pending_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("mux_pending_count");

    for count in [0, 10, 100, 1000] {
        let mux = Multiplexer::new();

        // Register requests (keep receivers alive)
        let _receivers: Vec<_> = (0..count).map(|_| mux.register().1).collect();

        group.bench_with_input(BenchmarkId::from_parameter(count), &(), |b, _| {
            b.iter(|| {
                let pending = mux.pending_count();
                black_box(pending);
            });
        });
    }

    group.finish();
}

/// Benchmark sequential register/dispatch simulating multiple requests.
/// Note: Multiplexer is designed for single-threaded use within ntex's worker model.
fn bench_mux_sequential_requests(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("mux_sequential_requests");

    for count in [4, 16, 64] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.to_async(&rt).iter(|| async {
                let mux = Multiplexer::new();

                for _ in 0..count {
                    let (id, rx) = mux.register();

                    // Simulate immediate response
                    let mut response = AlignedVec::new();
                    response.extend_from_slice(b"response");
                    mux.dispatch(id, response);

                    let result = rx.await.unwrap();
                    let _ = black_box(result);
                }
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_mux_register_dispatch,
    bench_mux_concurrent_pending,
    bench_mux_id_generation,
    bench_mux_cancel,
    bench_mux_dispatch_miss,
    bench_mux_pending_count,
    bench_mux_sequential_requests,
);

criterion_main!(benches);
