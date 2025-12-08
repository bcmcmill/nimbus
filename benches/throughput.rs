//! End-to-end throughput benchmarks.
//!
//! These benchmarks measure full RPC path performance including
//! serialization, framing, and simulated network I/O.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nimbus_codec::NimbusCodec;
use nimbus_core::{MessageType, RpcEnvelope};
use ntex_bytes::BytesMut;
use ntex_codec::Decoder as _;
use rkyv::rancor::Error as RkyvError;

/// Benchmark full RPC serialization path (serialize + frame).
fn bench_rpc_serialize_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("rpc_serialize_path");
    let codec = NimbusCodec::new();

    for payload_size in [64, 256, 1024, 10_240, 102_400] {
        let envelope = RpcEnvelope {
            request_id: 1,
            message_type: MessageType::Request,
            service: "BenchService".to_string(),
            method: "bench_method".to_string(),
            payload: vec![0xABu8; payload_size],
        };

        group.throughput(Throughput::Bytes(payload_size as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(payload_size),
            &envelope,
            |b, envelope| {
                let mut frame_buf = BytesMut::with_capacity(payload_size + 256);

                b.iter(|| {
                    // Step 1: Serialize with rkyv
                    let bytes = rkyv::to_bytes::<RkyvError>(black_box(envelope)).unwrap();

                    // Step 2: Frame encode
                    frame_buf.clear();
                    codec.encode_slice(&bytes, &mut frame_buf).unwrap();

                    black_box(&frame_buf);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark full RPC deserialization path (decode frame + deserialize).
fn bench_rpc_deserialize_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("rpc_deserialize_path");
    let codec = NimbusCodec::new();

    for payload_size in [64, 256, 1024, 10_240, 102_400] {
        let envelope = RpcEnvelope {
            request_id: 1,
            message_type: MessageType::Response,
            service: "BenchService".to_string(),
            method: "bench_method".to_string(),
            payload: vec![0xABu8; payload_size],
        };

        // Pre-serialize and frame
        let bytes = rkyv::to_bytes::<RkyvError>(&envelope).unwrap();
        let mut framed = BytesMut::with_capacity(bytes.len() + 4);
        codec.encode_slice(&bytes, &mut framed).unwrap();
        let framed_len = framed.len();

        group.throughput(Throughput::Bytes(framed_len as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(payload_size),
            &framed,
            |b, framed| {
                b.iter(|| {
                    // Step 1: Decode frame
                    let mut buf = framed.clone();
                    let aligned = codec.decode(black_box(&mut buf)).unwrap().unwrap();

                    // Step 2: Zero-copy access
                    let archived = rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(&aligned)
                        .unwrap();

                    black_box(archived.request_id);
                    black_box(&archived.service);
                    black_box(&archived.method);
                    black_box(archived.payload.len());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark full roundtrip (serialize -> frame -> decode -> access).
fn bench_rpc_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("rpc_roundtrip");
    let codec = NimbusCodec::new();

    for payload_size in [64, 256, 1024, 10_240] {
        let envelope = RpcEnvelope {
            request_id: 1,
            message_type: MessageType::Request,
            service: "BenchService".to_string(),
            method: "bench_method".to_string(),
            payload: vec![0xABu8; payload_size],
        };

        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::from_parameter(payload_size),
            &envelope,
            |b, envelope| {
                let mut frame_buf = BytesMut::with_capacity(payload_size + 256);

                b.iter(|| {
                    // Serialize
                    let bytes = rkyv::to_bytes::<RkyvError>(black_box(envelope)).unwrap();

                    // Frame
                    frame_buf.clear();
                    codec.encode_slice(&bytes, &mut frame_buf).unwrap();

                    // Decode
                    let aligned = codec.decode(&mut frame_buf).unwrap().unwrap();

                    // Access
                    let archived =
                        rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(&aligned).unwrap();

                    black_box(archived.request_id);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark request/response pair (simulates full RPC call).
fn bench_rpc_request_response(c: &mut Criterion) {
    let mut group = c.benchmark_group("rpc_request_response");
    let codec = NimbusCodec::new();

    for payload_size in [64, 256, 1024] {
        // Create request
        let request = RpcEnvelope {
            request_id: 1,
            message_type: MessageType::Request,
            service: "BenchService".to_string(),
            method: "bench_method".to_string(),
            payload: vec![0xABu8; payload_size],
        };

        // Create response
        let response = RpcEnvelope {
            request_id: 1,
            message_type: MessageType::Response,
            service: "BenchService".to_string(),
            method: "bench_method".to_string(),
            payload: vec![0xCDu8; payload_size],
        };

        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::from_parameter(payload_size),
            &(request, response),
            |b, (request, response)| {
                let mut req_buf = BytesMut::with_capacity(payload_size + 256);
                let mut resp_buf = BytesMut::with_capacity(payload_size + 256);

                b.iter(|| {
                    // Client: serialize request
                    let req_bytes = rkyv::to_bytes::<RkyvError>(black_box(request)).unwrap();
                    req_buf.clear();
                    codec.encode_slice(&req_bytes, &mut req_buf).unwrap();

                    // "Server": decode request
                    let req_aligned = codec.decode(&mut req_buf).unwrap().unwrap();
                    let _req_archived =
                        rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(&req_aligned)
                            .unwrap();

                    // Server: serialize response
                    let resp_bytes = rkyv::to_bytes::<RkyvError>(black_box(response)).unwrap();
                    resp_buf.clear();
                    codec.encode_slice(&resp_bytes, &mut resp_buf).unwrap();

                    // Client: decode response
                    let resp_aligned = codec.decode(&mut resp_buf).unwrap().unwrap();
                    let resp_archived =
                        rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(&resp_aligned)
                            .unwrap();

                    black_box(resp_archived.request_id);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark throughput (requests per second estimate).
fn bench_throughput_estimate(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_estimate");
    let codec = NimbusCodec::new();

    // Minimal overhead RPC (ping-like)
    let envelope = RpcEnvelope {
        request_id: 1,
        message_type: MessageType::Request,
        service: "Ping".to_string(),
        method: "ping".to_string(),
        payload: vec![],
    };

    group.throughput(Throughput::Elements(1000));

    group.bench_function("1000_minimal_rpcs", |b| {
        let mut frame_buf = BytesMut::with_capacity(256);

        b.iter(|| {
            for _i in 0..1000u64 {
                // Serialize
                let bytes = rkyv::to_bytes::<RkyvError>(black_box(&envelope)).unwrap();

                // Frame
                frame_buf.clear();
                codec.encode_slice(&bytes, &mut frame_buf).unwrap();

                // Decode
                let aligned = codec.decode(&mut frame_buf).unwrap().unwrap();

                // Access
                let archived =
                    rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(&aligned).unwrap();

                black_box(archived.request_id);
            }
        });
    });

    group.finish();
}

/// Measure raw latency for a single RPC operation.
fn bench_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency");
    group.sample_size(1000); // More samples for accurate latency measurement

    let codec = NimbusCodec::new();

    let envelope = RpcEnvelope {
        request_id: 1,
        message_type: MessageType::Request,
        service: "LatencyTest".to_string(),
        method: "test".to_string(),
        payload: vec![0u8; 64],
    };

    // Pre-serialize for decode-only benchmark
    let bytes = rkyv::to_bytes::<RkyvError>(&envelope).unwrap();
    let mut framed = BytesMut::with_capacity(bytes.len() + 4);
    codec.encode_slice(&bytes, &mut framed).unwrap();

    group.bench_function("serialize_only", |b| {
        b.iter(|| {
            let bytes = rkyv::to_bytes::<RkyvError>(black_box(&envelope)).unwrap();
            black_box(bytes);
        });
    });

    group.bench_function("deserialize_only", |b| {
        b.iter(|| {
            let mut buf = framed.clone();
            let aligned = codec.decode(black_box(&mut buf)).unwrap().unwrap();
            let archived =
                rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(&aligned).unwrap();
            black_box(archived);
        });
    });

    group.bench_function("full_roundtrip", |b| {
        let mut frame_buf = BytesMut::with_capacity(256);
        b.iter(|| {
            // Serialize
            let bytes = rkyv::to_bytes::<RkyvError>(black_box(&envelope)).unwrap();

            // Frame
            frame_buf.clear();
            codec.encode_slice(&bytes, &mut frame_buf).unwrap();

            // Decode + access
            let aligned = codec.decode(&mut frame_buf).unwrap().unwrap();
            let archived =
                rkyv::access::<nimbus_core::ArchivedRpcEnvelope, RkyvError>(&aligned).unwrap();

            black_box(archived);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_rpc_serialize_path,
    bench_rpc_deserialize_path,
    bench_rpc_roundtrip,
    bench_rpc_request_response,
    bench_throughput_estimate,
    bench_latency,
);

criterion_main!(benches);
