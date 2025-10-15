//! Benchmarks comparing Borsh vs SSZ serialization for identifier types.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use ssz::{Decode, Encode};
// Suppress unused crate warnings for dependencies used by other benchmarks
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used by other benchmarks in this package"
)]
use strata_checkpoint_types as _;
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used by other benchmarks in this package"
)]
use strata_checkpoint_types_ssz as _;
use strata_identifiers as borsh_types;
use strata_identifiers_ssz as ssz_types;
use tree_hash::TreeHash;

// ============================================================================
// L1BlockCommitment Benchmarks
// ============================================================================

fn bench_l1_commitment(c: &mut Criterion) {
    let mut group = c.benchmark_group("L1BlockCommitment");

    let borsh_val = borsh_types::L1BlockCommitment::from_height_u64(
        12345,
        borsh_types::L1BlockId::from(borsh_types::Buf32::from([0xABu8; 32])),
    )
    .unwrap();

    group.throughput(Throughput::Elements(1));
    group.bench_function("borsh_serialize", |b| {
        b.iter(|| {
            let bytes = borsh::to_vec(black_box(&borsh_val)).unwrap();
            black_box(bytes);
        })
    });

    group.bench_function("borsh_deserialize", |b| {
        let bytes = borsh::to_vec(&borsh_val).unwrap();
        b.iter(|| {
            let decoded: borsh_types::L1BlockCommitment =
                borsh::from_slice(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_val: ssz_types::L1BlockCommitment = borsh_val.into();

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded = ssz_types::L1BlockCommitment::from_ssz_bytes(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    group.bench_function("ssz_merkleize", |b| {
        b.iter(|| {
            let root = black_box(&ssz_val).tree_hash_root();
            black_box(root);
        })
    });

    group.finish();
}

// ============================================================================
// EpochCommitment Benchmarks
// ============================================================================

fn bench_epoch_commitment(c: &mut Criterion) {
    let mut group = c.benchmark_group("EpochCommitment");

    let borsh_val = borsh_types::EpochCommitment::new(
        10,
        1000,
        borsh_types::L2BlockId::from(borsh_types::Buf32::from([0xEFu8; 32])),
    );

    group.throughput(Throughput::Elements(1));
    group.bench_function("borsh_serialize", |b| {
        b.iter(|| {
            let bytes = borsh::to_vec(black_box(&borsh_val)).unwrap();
            black_box(bytes);
        })
    });

    group.bench_function("borsh_deserialize", |b| {
        let bytes = borsh::to_vec(&borsh_val).unwrap();
        b.iter(|| {
            let decoded: borsh_types::EpochCommitment =
                borsh::from_slice(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_val: ssz_types::EpochCommitment = borsh_val.into();

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded = ssz_types::EpochCommitment::from_ssz_bytes(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    group.bench_function("ssz_merkleize", |b| {
        b.iter(|| {
            let root = black_box(&ssz_val).tree_hash_root();
            black_box(root);
        })
    });

    group.finish();
}

// ============================================================================
// Batch Operations
// ============================================================================

fn bench_batch_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_100_L1BlockCommitments");
    group.throughput(Throughput::Elements(100));

    let borsh_vals: Vec<borsh_types::L1BlockCommitment> = (0..100)
        .map(|i| {
            borsh_types::L1BlockCommitment::from_height_u64(
                i,
                borsh_types::L1BlockId::from(borsh_types::Buf32::from([i as u8; 32])),
            )
            .unwrap()
        })
        .collect();

    group.bench_function("borsh_serialize", |b| {
        b.iter(|| {
            let serialized: Vec<Vec<u8>> = black_box(&borsh_vals)
                .iter()
                .map(|c| borsh::to_vec(c).unwrap())
                .collect();
            black_box(serialized);
        })
    });

    let ssz_vals: Vec<ssz_types::L1BlockCommitment> =
        borsh_vals.iter().map(|c| (*c).into()).collect();

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let serialized: Vec<Vec<u8>> = black_box(&ssz_vals)
                .iter()
                .map(|c| c.as_ssz_bytes())
                .collect();
            black_box(serialized);
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_l1_commitment,
    bench_epoch_commitment,
    bench_batch_serialization
);
criterion_main!(benches);
