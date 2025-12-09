//! Frame benchmarks - encoding/decoding performance.
//!
//! These benchmarks measure frame encoding and decoding throughput
//! at various payload sizes.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nimbus_codec::NimbusCodec;
use ntex_bytes::BytesMut;
use ntex_codec::Decoder as _;

/// Benchmark frame encoding at various sizes.
fn bench_frame_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_encode");
    let codec = NimbusCodec::new();

    for size in [64, 256, 1024, 10_240, 102_400, 1_048_576] {
        let payload = vec![0xABu8; size];
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &payload, |b, payload| {
            let mut buf = BytesMut::with_capacity(size + 4);
            b.iter(|| {
                buf.clear();
                codec.encode_slice(black_box(payload), &mut buf).unwrap();
                black_box(&buf);
            });
        });
    }

    group.finish();
}

/// Benchmark frame decoding at various sizes.
fn bench_frame_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_decode");
    let codec = NimbusCodec::new();

    for size in [64, 256, 1024, 10_240, 102_400, 1_048_576] {
        // Pre-encode the frame
        let payload = vec![0xABu8; size];
        let mut encoded = BytesMut::with_capacity(size + 4);
        codec.encode_slice(&payload, &mut encoded).unwrap();
        let encoded_len = encoded.len();

        group.throughput(Throughput::Bytes(encoded_len as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &encoded, |b, encoded| {
            b.iter(|| {
                let mut buf = encoded.clone();
                let frame = codec.decode(black_box(&mut buf)).unwrap().unwrap();
                black_box(frame);
            });
        });
    }

    group.finish();
}

/// Benchmark encode/decode roundtrip.
fn bench_frame_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_roundtrip");
    let codec = NimbusCodec::new();

    for size in [64, 256, 1024, 10_240, 102_400] {
        let payload = vec![0xABu8; size];
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &payload, |b, payload| {
            let mut buf = BytesMut::with_capacity(size + 4);
            b.iter(|| {
                buf.clear();
                codec.encode_slice(black_box(payload), &mut buf).unwrap();
                let frame = codec.decode(&mut buf).unwrap().unwrap();
                black_box(frame);
            });
        });
    }

    group.finish();
}

/// Benchmark multiple sequential frames in a single buffer.
fn bench_frame_multiple(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_multiple");
    let codec = NimbusCodec::new();

    for count in [4, 16, 64] {
        let payload = vec![0xABu8; 256];

        // Pre-encode multiple frames
        let mut encoded = BytesMut::with_capacity((256 + 4) * count);
        for _ in 0..count {
            codec.encode_slice(&payload, &mut encoded).unwrap();
        }

        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("decode_batch", count),
            &encoded,
            |b, encoded| {
                b.iter(|| {
                    let mut buf = encoded.clone();
                    let mut decoded = 0;
                    while let Some(frame) = codec.decode(black_box(&mut buf)).unwrap() {
                        black_box(&frame);
                        decoded += 1;
                    }
                    assert_eq!(decoded, count);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark alignment copy overhead.
fn bench_alignment_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("alignment_overhead");
    let codec = NimbusCodec::new();

    // Compare decode (which copies to aligned buffer) vs just copying
    for size in [1024, 10_240, 102_400] {
        let payload = vec![0xABu8; size];
        let mut encoded = BytesMut::with_capacity(size + 4);
        codec.encode_slice(&payload, &mut encoded).unwrap();

        group.throughput(Throughput::Bytes(size as u64));

        // Baseline: simple Vec copy
        group.bench_with_input(
            BenchmarkId::new("vec_copy", size),
            &payload,
            |b, payload| {
                b.iter(|| {
                    let mut v = Vec::with_capacity(payload.len());
                    v.extend_from_slice(black_box(payload));
                    black_box(v);
                });
            },
        );

        // Aligned decode
        group.bench_with_input(
            BenchmarkId::new("aligned_decode", size),
            &encoded,
            |b, encoded| {
                b.iter(|| {
                    let mut buf = encoded.clone();
                    let frame = codec.decode(black_box(&mut buf)).unwrap().unwrap();
                    black_box(frame);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_frame_encode,
    bench_frame_decode,
    bench_frame_roundtrip,
    bench_frame_multiple,
    bench_alignment_overhead,
);

criterion_main!(benches);
