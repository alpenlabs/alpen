//! Benchmarks for [`L2BlockDatabase`] trait implementations.
//!
//! This module benchmarks the core operations of the L2BlockDatabase trait using RocksDB
//! as the storage backend. The benchmarks test various payload sizes and operation patterns
//! to measure performance characteristics.

use std::hint::black_box;

use alpen_benchmarks::db::{create_temp_rocksdb, default_db_ops_config};
use arbitrary::Arbitrary;
// Suppress unused crate warnings
#[allow(unused_imports)]
use bitcoin as _;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
#[allow(unused_imports)]
use rockbound as _;
use strata_db::traits::{BlockStatus, L2BlockDatabase};
use strata_db_store_rocksdb::l2::db::L2Db;
#[allow(unused_imports)]
use strata_primitives as _;
#[allow(unused_imports)]
use strata_state as _;
use strata_state::prelude::*;
use tempfile::TempDir;

/// Payload operation counts to test across benchmarks.
const PAYLOAD_SIZES: &[usize] = &[1, 2, 3, 5, 10, 20, 50, 100, 250, 1_000];

/// Block counts to test for chain operations.
const BLOCK_COUNTS: &[usize] = &[1, 2, 3, 5, 10, 20, 50, 100, 250, 1_000];

/// Benchmark setup helper that creates a temporary [`L2Db`] instance.
struct BenchSetup {
    db: L2Db,
    _temp_dir: TempDir,
}

impl BenchSetup {
    /// Creates a new [`BenchSetup`] with a temporary `RocksDB` instance.
    fn new() -> Self {
        let (rocksdb, temp_dir) = create_temp_rocksdb();
        let ops_config = default_db_ops_config();
        let db = L2Db::new(rocksdb, ops_config);

        Self {
            db,
            _temp_dir: temp_dir,
        }
    }
}

/// Benchmark [`L2BlockDatabase::put_block_data`] with varying payload sizes.
fn bench_put_block_data(c: &mut Criterion) {
    let mut group = c.benchmark_group("l2_put_block_data");

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(payload_ops as u64));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();
                        // Generate data using Arbitrary with a deterministic seed based on
                        // payload_ops
                        let seed_data = vec![payload_ops as u8; 1024]; // Use payload_ops as seed
                        let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                        let bundle =
                            strata_state::block::L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                        (setup, bundle)
                    },
                    |(setup, bundle)| black_box(setup.db.put_block_data(bundle)).unwrap(),
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L2BlockDatabase::get_block_data`] with varying payload sizes.
fn bench_get_block_data(c: &mut Criterion) {
    let mut group = c.benchmark_group("l2_get_block_data");

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(payload_ops as u64));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();
                        // Generate data using Arbitrary with a deterministic seed based on
                        // payload_ops
                        let seed_data = vec![(payload_ops + 1) as u8; 1024]; // Use payload_ops+1 as seed to differentiate
                        let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                        let bundle =
                            strata_state::block::L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                        let block_id = bundle.block().header().get_blockid();

                        // Pre-populate database
                        setup.db.put_block_data(bundle).unwrap();

                        (setup, block_id)
                    },
                    |(setup, block_id)| black_box(setup.db.get_block_data(block_id)).unwrap(),
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L2BlockDatabase::set_block_status`] with varying payload sizes.
fn bench_set_block_status(c: &mut Criterion) {
    let mut group = c.benchmark_group("l2_set_block_status");

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();
                        // Generate data using Arbitrary with a deterministic seed based on
                        // payload_ops
                        let seed_data = vec![(payload_ops + 2) as u8; 1024]; // Use payload_ops+2 as seed to differentiate
                        let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                        let bundle =
                            strata_state::block::L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                        let block_id = bundle.block().header().get_blockid();

                        // Pre-populate database
                        setup.db.put_block_data(bundle).unwrap();

                        (setup, block_id)
                    },
                    |(setup, block_id)| {
                        black_box(setup.db.set_block_status(block_id, BlockStatus::Valid)).unwrap()
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L2BlockDatabase::get_block_status`] with varying payload sizes.
fn bench_get_block_status(c: &mut Criterion) {
    let mut group = c.benchmark_group("l2_get_block_status");

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();
                        // Generate data using Arbitrary with a deterministic seed based on
                        // payload_ops
                        let seed_data = vec![(payload_ops + 3) as u8; 1024]; // Use payload_ops+3 as seed to differentiate
                        let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                        let bundle =
                            strata_state::block::L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                        let block_id = bundle.block().header().get_blockid();

                        // Pre-populate database with status
                        setup.db.put_block_data(bundle).unwrap();
                        setup
                            .db
                            .set_block_status(block_id, BlockStatus::Valid)
                            .unwrap();

                        (setup, block_id)
                    },
                    |(setup, block_id)| black_box(setup.db.get_block_status(block_id)).unwrap(),
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L2BlockDatabase::get_blocks_at_height`] with competing blocks.
fn bench_get_blocks_at_height(c: &mut Criterion) {
    let mut group = c.benchmark_group("l2_get_blocks_at_height");

    for &block_count in &[1, 2, 3, 5] {
        // Test with fewer competing blocks
        group.throughput(Throughput::Elements(block_count as u64));

        group.bench_with_input(
            BenchmarkId::new("competing_blocks", block_count),
            &block_count,
            |b, &block_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();

                        // Generate competing blocks using Arbitrary
                        let mut blocks = Vec::new();
                        for i in 0..block_count {
                            let seed_data = vec![(i + 100) as u8; 1024]; // Different seed per competing block
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle =
                                strata_state::block::L2BlockBundle::arbitrary(&mut unstructured)
                                    .expect("Failed to generate L2BlockBundle");
                            blocks.push(bundle);
                        }
                        let height = blocks[0].block().header().slot();

                        // Pre-populate database with competing blocks
                        for block in &blocks {
                            setup.db.put_block_data(block.clone()).unwrap();
                        }

                        (setup, height)
                    },
                    |(setup, height)| black_box(setup.db.get_blocks_at_height(height)).unwrap(),
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L2BlockDatabase::get_tip_block`] with varying chain lengths.
fn bench_get_tip_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("l2_get_tip_block");

    for &block_count in BLOCK_COUNTS {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("chain_length", block_count),
            &block_count,
            |b, &block_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();

                        // Create chain of blocks with valid status using Arbitrary
                        let mut blocks = Vec::new();
                        for i in 0..block_count {
                            let seed_data = vec![(i + 200) as u8; 1024]; // Different seed per block in chain
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle =
                                strata_state::block::L2BlockBundle::arbitrary(&mut unstructured)
                                    .expect("Failed to generate L2BlockBundle");
                            blocks.push(bundle);
                        }

                        for block in &blocks {
                            let block_id = block.block().header().get_blockid();
                            setup.db.put_block_data(block.clone()).unwrap();
                            setup
                                .db
                                .set_block_status(block_id, BlockStatus::Valid)
                                .unwrap();
                        }

                        setup
                    },
                    |setup| black_box(setup.db.get_tip_block()).unwrap(),
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_put_block_data,
    bench_get_block_data,
    bench_set_block_status,
    bench_get_block_status,
    bench_get_blocks_at_height,
    bench_get_tip_block,
);

criterion_main!(benches);
