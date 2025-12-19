//! Benchmarks for [`L1Database`] trait implementations.
//!
//! This module benchmarks the core operations of the [`L1Database`] trait with
//! feature-gated support for different database backends (Sled by default, RocksDB optional).

use std::hint::black_box;

use alpen_benchmarks::db::DatabaseBackend;
use arbitrary::Arbitrary;
// Suppress unused crate warnings
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use bitcoin as _;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use strata_asm_manifest_types::AsmManifest;
use strata_db_types::traits::L1Database;
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_ol_chain_types as _;
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_primitives as _;
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used for benchmarking"
)]
use strata_state as _;
use tempfile::TempDir;
// Feature-gated imports
#[cfg(feature = "sled")]
use {
    alpen_benchmarks::db::sled::{create_temp_sled, default_sled_ops_config},
    strata_db_store_sled::l1::db::L1DBSled,
    typed_sled as _,
};

/// Transaction counts to test across benchmarks.
const TX_COUNTS: &[usize] = &[1, 2, 3, 5, 10, 20, 50, 100, 250, 1_000];

/// Block counts to test for chain operations.
const BLOCK_COUNTS: &[usize] = &[1, 2, 3, 5, 10, 20, 50, 100, 250, 1_000];

/// Benchmark setup for Sled L1 database.
#[cfg(feature = "sled")]
struct L1BenchSetupSled {
    db: L1DBSled,
    _temp_dir: TempDir,
}

#[cfg(feature = "sled")]
impl L1BenchSetupSled {
    fn new() -> Self {
        let (sled_db, temp_dir) = create_temp_sled();
        let ops_config = default_sled_ops_config();
        let db = L1DBSled::new(sled_db, ops_config).expect("Failed to create L1DBSled");
        Self {
            db,
            _temp_dir: temp_dir,
        }
    }
}

/// Generic benchmark implementation for [`L1Database::put_block_data`].
fn bench_put_block_data_impl(backend: DatabaseBackend, c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("l1_put_block_data_{}", backend.name()));

    for &tx_count in TX_COUNTS {
        group.throughput(Throughput::Elements(tx_count as u64));

        group.bench_with_input(
            BenchmarkId::new("tx_count", tx_count),
            &tx_count,
            |b, &tx_count| match backend {
                #[cfg(feature = "sled")]
                DatabaseBackend::Sled => {
                    b.iter_with_setup(
                        || {
                            let setup = L1BenchSetupSled::new();
                            let seed_data = vec![tx_count as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let manifest = AsmManifest::arbitrary(&mut unstructured)
                                .expect("Failed to generate AsmManifest");
                            (setup, manifest)
                        },
                        |(setup, manifest)| {
                            black_box(setup.db.put_block_data(manifest, 0)).unwrap()
                        },
                    );
                }
            },
        );
    }

    group.finish();
}

/// Generic benchmark implementation for [`L1Database::get_block_manifest`].
fn bench_get_block_manifest_impl(backend: DatabaseBackend, c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("l1_get_block_manifest_{}", backend.name()));

    for &tx_count in TX_COUNTS {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("tx_count", tx_count),
            &tx_count,
            |b, &tx_count| match backend {
                #[cfg(feature = "sled")]
                DatabaseBackend::Sled => {
                    b.iter_with_setup(
                        || {
                            let setup = L1BenchSetupSled::new();
                            let seed_data = vec![(tx_count + 1) as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let manifest = AsmManifest::arbitrary(&mut unstructured)
                                .expect("Failed to generate AsmManifest");
                            let blockid = *manifest.blkid();
                            setup.db.put_block_data(manifest, 0).unwrap();
                            (setup, blockid)
                        },
                        |(setup, blockid)| black_box(setup.db.get_block_manifest(blockid)).unwrap(),
                    );
                }
            },
        );
    }

    group.finish();
}

/// Generic benchmark implementation for [`L1Database::get_canonical_blockid_at_height`].
fn bench_get_canonical_blockid_at_height_impl(backend: DatabaseBackend, c: &mut Criterion) {
    let mut group = c.benchmark_group(format!(
        "l1_get_canonical_blockid_at_height_{}",
        backend.name()
    ));

    for &block_count in BLOCK_COUNTS {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("block_count", block_count),
            &block_count,
            |b, &block_count| match backend {
                #[cfg(feature = "sled")]
                DatabaseBackend::Sled => {
                    b.iter_with_setup(
                        || {
                            let setup = L1BenchSetupSled::new();
                            let mut blocks = Vec::new();
                            for i in 0..block_count {
                                let seed_data = vec![(i % 256) as u8; 1024];
                                let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                                let manifest = AsmManifest::arbitrary(&mut unstructured)
                                    .expect("Failed to generate AsmManifest");
                                blocks.push(manifest);
                            }
                            for (i, block) in blocks.iter().enumerate() {
                                setup.db.put_block_data(block.clone(), i as u64).unwrap();
                                setup
                                    .db
                                    .set_canonical_chain_entry(i as u64, *block.blkid())
                                    .unwrap();
                            }
                            let target_height = (block_count / 2) as u64;
                            (setup, target_height)
                        },
                        |(setup, height)| {
                            black_box(setup.db.get_canonical_blockid_at_height(height)).unwrap()
                        },
                    );
                }
            },
        );
    }

    group.finish();
}

// Use the macro to generate benchmarks for all available backends
alpen_benchmarks::bench_all_backends!(bench_put_block_data, bench_put_block_data_impl);
alpen_benchmarks::bench_all_backends!(bench_get_block_manifest, bench_get_block_manifest_impl);
alpen_benchmarks::bench_all_backends!(
    bench_get_canonical_blockid_at_height,
    bench_get_canonical_blockid_at_height_impl
);

criterion_group!(
    benches,
    bench_put_block_data,
    bench_get_block_manifest,
    bench_get_canonical_blockid_at_height,
);

criterion_main!(benches);
