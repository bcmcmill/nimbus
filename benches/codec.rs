//! Codec benchmarks - rkyv serialization performance.
//!
//! These benchmarks measure serialization/deserialization throughput
//! and compare rkyv's zero-copy approach vs traditional serde.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nimbus_codec::AlignedBufferPool;
use nimbus_core::{MessageType, RpcEnvelope};
use rkyv::{Archive, Deserialize, Serialize, rancor::Error as RkyvError};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};

/// Test message for benchmarking serialization.
#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
#[rkyv(derive(Debug))]
struct BenchMessage {
    id: u64,
    name: String,
    payload: Vec<u8>,
}

/// Serde equivalent for comparison.
#[derive(Debug, Clone, SerdeSerialize, SerdeDeserialize)]
struct BenchMessageSerde {
    id: u64,
    name: String,
    payload: Vec<u8>,
}

fn create_message(payload_size: usize) -> BenchMessage {
    BenchMessage {
        id: 12345,
        name: "benchmark_message".to_string(),
        payload: vec![0xAB; payload_size],
    }
}

fn create_message_serde(payload_size: usize) -> BenchMessageSerde {
    BenchMessageSerde {
        id: 12345,
        name: "benchmark_message".to_string(),
        payload: vec![0xAB; payload_size],
    }
}

/// Benchmark rkyv serialization at various payload sizes.
fn bench_rkyv_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("rkyv_serialize");

    for size in [64, 256, 1024, 10_240, 102_400, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        let msg = create_message(size);

        group.bench_with_input(BenchmarkId::from_parameter(size), &msg, |b, msg| {
            b.iter(|| {
                let bytes = rkyv::to_bytes::<RkyvError>(black_box(msg)).unwrap();
                black_box(bytes)
            });
        });
    }

    group.finish();
}

/// Benchmark rkyv deserialization (zero-copy access).
fn bench_rkyv_deserialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("rkyv_deserialize");

    for size in [64, 256, 1024, 10_240, 102_400, 1_048_576] {
        let msg = create_message(size);
        let bytes = rkyv::to_bytes::<RkyvError>(&msg).unwrap();
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &bytes, |b, bytes| {
            b.iter(|| {
                // Zero-copy access - this is O(1)!
                let archived =
                    rkyv::access::<ArchivedBenchMessage, RkyvError>(black_box(bytes)).unwrap();
                black_box(archived.id);
                black_box(&archived.name);
                black_box(archived.payload.len());
            });
        });
    }

    group.finish();
}

/// Benchmark serde_json serialization for comparison.
fn bench_serde_json_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("serde_json_serialize");

    for size in [64, 256, 1024, 10_240, 102_400] {
        group.throughput(Throughput::Bytes(size as u64));
        let msg = create_message_serde(size);

        group.bench_with_input(BenchmarkId::from_parameter(size), &msg, |b, msg| {
            b.iter(|| {
                let json = serde_json::to_vec(black_box(msg)).unwrap();
                black_box(json)
            });
        });
    }

    group.finish();
}

/// Benchmark serde_json deserialization for comparison.
fn bench_serde_json_deserialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("serde_json_deserialize");

    for size in [64, 256, 1024, 10_240, 102_400] {
        let msg = create_message_serde(size);
        let json = serde_json::to_vec(&msg).unwrap();
        group.throughput(Throughput::Bytes(json.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &json, |b, json| {
            b.iter(|| {
                let msg: BenchMessageSerde = serde_json::from_slice(black_box(json)).unwrap();
                black_box(msg.id);
                black_box(&msg.name);
                black_box(msg.payload.len());
            });
        });
    }

    group.finish();
}

/// Benchmark RpcEnvelope serialization (real-world message type).
fn bench_rpc_envelope(c: &mut Criterion) {
    let mut group = c.benchmark_group("rpc_envelope");

    for payload_size in [64, 256, 1024, 10_240] {
        let envelope = RpcEnvelope {
            request_id: 1,
            message_type: MessageType::Request,
            service: "TestService".to_string(),
            method: "test_method".to_string(),
            payload: vec![0u8; payload_size],
        };

        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("serialize", payload_size),
            &envelope,
            |b, envelope| {
                b.iter(|| {
                    let bytes = rkyv::to_bytes::<RkyvError>(black_box(envelope)).unwrap();
                    black_box(bytes)
                });
            },
        );

        let bytes = rkyv::to_bytes::<RkyvError>(&envelope).unwrap();
        group.bench_with_input(
            BenchmarkId::new("deserialize", payload_size),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let archived = rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(
                        black_box(bytes),
                    )
                    .unwrap();
                    black_box(archived.request_id);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark buffer pool acquire/release.
fn bench_buffer_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("buffer_pool");

    let pool = AlignedBufferPool::with_config(1024, 16);

    group.bench_function("acquire_release", |b| {
        b.iter(|| {
            let buffer = pool.acquire();
            black_box(&buffer);
            // buffer is automatically released on drop
        });
    });

    // Pre-warm the pool
    {
        let buffers: Vec<_> = (0..16).map(|_| pool.acquire()).collect();
        drop(buffers);
    }

    group.bench_function("acquire_release_warmed", |b| {
        b.iter(|| {
            let buffer = pool.acquire();
            black_box(&buffer);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_rkyv_serialize,
    bench_rkyv_deserialize,
    bench_serde_json_serialize,
    bench_serde_json_deserialize,
    bench_rpc_envelope,
    bench_buffer_pool,
);

criterion_main!(benches);
