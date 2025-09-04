//! Benchmarks for [`L1Database`] trait implementations.
//!
//! This module benchmarks the core operations of the [`L1Database`] trait using `RocksDB`
//! as the storage backend. The benchmarks test various data sizes and operation patterns
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
use strata_db::traits::L1Database;
use strata_db_store_rocksdb::l1::db::L1Db;
#[allow(unused_imports)]
use strata_primitives as _;
#[allow(unused_imports)]
use strata_state as _;
use tempfile::TempDir;

/// Transaction counts to test across benchmarks.
const TX_COUNTS: &[usize] = &[1, 2, 3, 5, 10, 20, 50, 100, 250, 1_000];

/// Block counts to test for chain operations.
const BLOCK_COUNTS: &[usize] = &[1, 2, 3, 5, 10, 20, 50, 100, 250, 1_000];

/// Benchmark setup helper that creates a temporary [`L1Db`] instance.
struct BenchSetup {
    db: L1Db,
    _temp_dir: TempDir,
}

impl BenchSetup {
    /// Creates a new [`BenchSetup`] with a temporary `RocksDB` instance.
    fn new() -> Self {
        let (rocksdb, temp_dir) = create_temp_rocksdb();
        let ops_config = default_db_ops_config();
        let db = L1Db::new(rocksdb, ops_config);

        Self {
            db,
            _temp_dir: temp_dir,
        }
    }
}

/// Benchmark [`L1Database::put_block_data`] with varying transaction counts.
fn bench_put_block_data(c: &mut Criterion) {
    let mut group = c.benchmark_group("l1_put_block_data");

    for &tx_count in TX_COUNTS {
        group.throughput(Throughput::Elements(tx_count as u64));

        group.bench_with_input(
            BenchmarkId::new("tx_count", tx_count),
            &tx_count,
            |b, &tx_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();
                        // Generate data using Arbitrary with a deterministic seed based on tx_count
                        let seed_data = vec![tx_count as u8; 1024]; // Use tx_count as seed
                        let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                        let manifest =
                            strata_primitives::l1::L1BlockManifest::arbitrary(&mut unstructured)
                                .expect("Failed to generate L1BlockManifest");
                        (setup, manifest)
                    },
                    |(setup, manifest)| black_box(setup.db.put_block_data(manifest)).unwrap(),
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L1Database::get_block_manifest`] with varying transaction counts.
fn bench_get_block_manifest(c: &mut Criterion) {
    let mut group = c.benchmark_group("l1_get_block_manifest");

    for &tx_count in TX_COUNTS {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("tx_count", tx_count),
            &tx_count,
            |b, &tx_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();
                        // Generate data using Arbitrary with a deterministic seed based on tx_count
                        let seed_data = vec![(tx_count + 1) as u8; 1024]; // Use tx_count+1 as seed to differentiate from previous
                        let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                        let manifest =
                            strata_primitives::l1::L1BlockManifest::arbitrary(&mut unstructured)
                                .expect("Failed to generate L1BlockManifest");
                        let blockid = *manifest.blkid();

                        // Pre-populate database
                        setup.db.put_block_data(manifest).unwrap();

                        (setup, blockid)
                    },
                    |(setup, blockid)| black_box(setup.db.get_block_manifest(blockid)).unwrap(),
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L1Database::get_canonical_blockid_at_height`] with varying chain lengths.
fn bench_get_canonical_blockid_at_height(c: &mut Criterion) {
    let mut group = c.benchmark_group("l1_get_canonical_blockid_at_height");

    for &block_count in BLOCK_COUNTS {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("block_count", block_count),
            &block_count,
            |b, &block_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();

                        // Pre-populate chain using Arbitrary
                        let mut blocks = Vec::new();
                        for i in 0..block_count {
                            let seed_data = vec![(i % 256) as u8; 1024]; // Different seed per block
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let manifest = strata_primitives::l1::L1BlockManifest::arbitrary(
                                &mut unstructured,
                            )
                            .expect("Failed to generate L1BlockManifest");
                            blocks.push(manifest);
                        }

                        for (i, block) in blocks.iter().enumerate() {
                            setup.db.put_block_data(block.clone()).unwrap();
                            setup
                                .db
                                .set_canonical_chain_entry(i as u64, *block.blkid())
                                .unwrap();
                        }

                        let target_height = (block_count / 2) as u64; // Query middle block
                        (setup, target_height)
                    },
                    |(setup, height)| {
                        black_box(setup.db.get_canonical_blockid_at_height(height)).unwrap()
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L1Database::get_block_txs`] with varying transaction counts.
fn bench_get_block_txs(c: &mut Criterion) {
    let mut group = c.benchmark_group("l1_get_block_txs");

    for &tx_count in TX_COUNTS {
        group.throughput(Throughput::Elements(tx_count as u64));

        group.bench_with_input(
            BenchmarkId::new("tx_count", tx_count),
            &tx_count,
            |b, &tx_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();
                        // Generate data using Arbitrary with a deterministic seed based on tx_count
                        let seed_data = vec![(tx_count + 2) as u8; 1024]; // Use tx_count+2 as seed to differentiate
                        let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                        let manifest =
                            strata_primitives::l1::L1BlockManifest::arbitrary(&mut unstructured)
                                .expect("Failed to generate L1BlockManifest");
                        let blockid = *manifest.blkid();

                        // Pre-populate database
                        setup.db.put_block_data(manifest).unwrap();

                        (setup, blockid)
                    },
                    |(setup, blockid)| black_box(setup.db.get_block_txs(blockid)).unwrap(),
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L1Database::set_canonical_chain_entry`] with varying chain lengths.
fn bench_set_canonical_chain_entry(c: &mut Criterion) {
    let mut group = c.benchmark_group("l1_set_canonical_chain_entry");

    for &block_count in BLOCK_COUNTS {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("chain_length", block_count),
            &block_count,
            |b, &block_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();

                        // Pre-populate with blocks using Arbitrary
                        let mut blocks = Vec::new();
                        for i in 0..block_count {
                            let seed_data = vec![(i + 100) as u8; 1024]; // Different seed per block, offset to avoid overlap
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let manifest = strata_primitives::l1::L1BlockManifest::arbitrary(
                                &mut unstructured,
                            )
                            .expect("Failed to generate L1BlockManifest");
                            blocks.push(manifest);
                        }

                        for block in &blocks {
                            setup.db.put_block_data(block.clone()).unwrap();
                        }

                        let target_block = &blocks[block_count / 2];
                        let height = target_block.height();
                        let blockid = *target_block.blkid();

                        (setup, height, blockid)
                    },
                    |(setup, height, blockid)| {
                        black_box(setup.db.set_canonical_chain_entry(height, blockid)).unwrap()
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmark [`L1Database::get_canonical_chain_tip`] with varying chain lengths.
fn bench_get_canonical_chain_tip(c: &mut Criterion) {
    let mut group = c.benchmark_group("l1_get_canonical_chain_tip");

    for &block_count in BLOCK_COUNTS {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("chain_length", block_count),
            &block_count,
            |b, &block_count| {
                b.iter_with_setup(
                    || {
                        let setup = BenchSetup::new();

                        // Pre-populate canonical chain using Arbitrary
                        let mut blocks = Vec::new();
                        for i in 0..block_count {
                            let seed_data = vec![(i + 200) as u8; 1024]; // Different seed per block, offset to avoid overlap
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let manifest = strata_primitives::l1::L1BlockManifest::arbitrary(
                                &mut unstructured,
                            )
                            .expect("Failed to generate L1BlockManifest");
                            blocks.push(manifest);
                        }

                        for (i, block) in blocks.iter().enumerate() {
                            setup.db.put_block_data(block.clone()).unwrap();
                            setup
                                .db
                                .set_canonical_chain_entry(i as u64, *block.blkid())
                                .unwrap();
                        }

                        setup
                    },
                    |setup| black_box(setup.db.get_canonical_chain_tip()).unwrap(),
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_put_block_data,
    bench_get_block_manifest,
    bench_get_canonical_blockid_at_height,
    bench_get_block_txs,
    bench_set_canonical_chain_entry,
    bench_get_canonical_chain_tip,
);

criterion_main!(benches);
