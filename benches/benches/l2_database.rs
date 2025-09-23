//! Benchmarks for [`L2BlockDatabase`] trait implementations.
//!
//! This module benchmarks the core operations of the L2BlockDatabase trait with
//! feature-gated support for different database backends (Sled by default, RocksDB optional).

use std::hint::black_box;

use alpen_benchmarks::db::DatabaseBackend;
use arbitrary::Arbitrary;
// Suppress unused crate warnings
#[allow(unused_imports)]
use bitcoin as _;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
#[allow(unused_imports)]
use strata_asm_types as _;
use strata_db::traits::{BlockStatus, L2BlockDatabase};
use strata_ol_chain_types::{L2BlockBundle, L2Header};
#[allow(unused_imports)]
use strata_primitives as _;
#[allow(unused_imports)]
use strata_state::prelude::*;
use tempfile::TempDir;
#[cfg(feature = "rocksdb")]
use {
    alpen_benchmarks::db::rocksdb::{create_temp_rocksdb, default_rocksdb_ops_config},
    rockbound as _,
    strata_db_store_rocksdb::l2::db::L2Db as L2DbRocks,
};
// Feature-gated imports
#[cfg(feature = "sled")]
use {
    alpen_benchmarks::db::sled::{create_temp_sled, default_sled_ops_config},
    strata_db_store_sled::l2::db::L2DBSled,
    typed_sled as _,
};

/// Payload operation counts to test across benchmarks.
const PAYLOAD_SIZES: &[usize] = &[1, 2, 3, 5, 10, 20, 50, 100, 250, 1_000];

/// Benchmark setup for Sled L2 database.
#[cfg(feature = "sled")]
struct L2BenchSetupSled {
    db: L2DBSled,
    _temp_dir: TempDir,
}

#[cfg(feature = "sled")]
impl L2BenchSetupSled {
    fn new() -> Self {
        let (sled_db, temp_dir) = create_temp_sled();
        let ops_config = default_sled_ops_config();
        let db = L2DBSled::new(sled_db, ops_config).expect("Failed to create L2DBSled");
        Self {
            db,
            _temp_dir: temp_dir,
        }
    }
}

/// Benchmark setup for RocksDB L2 database.
#[cfg(feature = "rocksdb")]
struct L2BenchSetupRocks {
    db: L2DbRocks,
    _temp_dir: TempDir,
}

#[cfg(feature = "rocksdb")]
impl L2BenchSetupRocks {
    fn new() -> Self {
        let (rocksdb, temp_dir) = create_temp_rocksdb();
        let ops_config = default_rocksdb_ops_config();
        let db = L2DbRocks::new(rocksdb, ops_config);
        Self {
            db,
            _temp_dir: temp_dir,
        }
    }
}

/// Generic benchmark implementation for [`L2BlockDatabase::put_block_data`].
fn bench_put_block_data_impl(backend: DatabaseBackend, c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("l2_put_block_data_{}", backend.name()));

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(payload_ops as u64));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| match backend {
                #[cfg(feature = "sled")]
                DatabaseBackend::Sled => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupSled::new();
                            let seed_data = vec![payload_ops as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            (setup, bundle)
                        },
                        |(setup, bundle)| black_box(setup.db.put_block_data(bundle)).unwrap(),
                    );
                }
                #[cfg(feature = "rocksdb")]
                DatabaseBackend::RocksDb => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupRocks::new();
                            let seed_data = vec![payload_ops as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            (setup, bundle)
                        },
                        |(setup, bundle)| black_box(setup.db.put_block_data(bundle)).unwrap(),
                    );
                }
            },
        );
    }

    group.finish();
}

/// Generic benchmark implementation for [`L2BlockDatabase::get_block_data`].
fn bench_get_block_data_impl(backend: DatabaseBackend, c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("l2_get_block_data_{}", backend.name()));

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(payload_ops as u64));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| match backend {
                #[cfg(feature = "sled")]
                DatabaseBackend::Sled => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupSled::new();
                            let seed_data = vec![(payload_ops + 1) as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            let block_id = bundle.block().header().get_blockid();
                            setup.db.put_block_data(bundle).unwrap();
                            (setup, block_id)
                        },
                        |(setup, block_id)| black_box(setup.db.get_block_data(block_id)).unwrap(),
                    );
                }
                #[cfg(feature = "rocksdb")]
                DatabaseBackend::RocksDb => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupRocks::new();
                            let seed_data = vec![(payload_ops + 1) as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            let block_id = bundle.block().header().get_blockid();
                            setup.db.put_block_data(bundle).unwrap();
                            (setup, block_id)
                        },
                        |(setup, block_id)| black_box(setup.db.get_block_data(block_id)).unwrap(),
                    );
                }
            },
        );
    }

    group.finish();
}

/// Generic benchmark implementation for [`L2BlockDatabase::set_block_status`].
fn bench_set_block_status_impl(backend: DatabaseBackend, c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("l2_set_block_status_{}", backend.name()));

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| match backend {
                #[cfg(feature = "sled")]
                DatabaseBackend::Sled => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupSled::new();
                            let seed_data = vec![(payload_ops + 2) as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            let block_id = bundle.block().header().get_blockid();
                            setup.db.put_block_data(bundle).unwrap();
                            (setup, block_id)
                        },
                        |(setup, block_id)| {
                            black_box(setup.db.set_block_status(block_id, BlockStatus::Valid))
                                .unwrap()
                        },
                    );
                }
                #[cfg(feature = "rocksdb")]
                DatabaseBackend::RocksDb => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupRocks::new();
                            let seed_data = vec![(payload_ops + 2) as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            let block_id = bundle.block().header().get_blockid();
                            setup.db.put_block_data(bundle).unwrap();
                            (setup, block_id)
                        },
                        |(setup, block_id)| {
                            black_box(setup.db.set_block_status(block_id, BlockStatus::Valid))
                                .unwrap()
                        },
                    );
                }
            },
        );
    }

    group.finish();
}

/// Generic benchmark implementation for [`L2BlockDatabase::get_block_status`].
fn bench_get_block_status_impl(backend: DatabaseBackend, c: &mut Criterion) {
    let mut group = c.benchmark_group(format!("l2_get_block_status_{}", backend.name()));

    for &payload_ops in PAYLOAD_SIZES {
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("payload_ops", payload_ops),
            &payload_ops,
            |b, &payload_ops| match backend {
                #[cfg(feature = "sled")]
                DatabaseBackend::Sled => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupSled::new();
                            let seed_data = vec![(payload_ops + 3) as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            let block_id = bundle.block().header().get_blockid();
                            setup.db.put_block_data(bundle).unwrap();
                            setup
                                .db
                                .set_block_status(block_id, BlockStatus::Valid)
                                .unwrap();
                            (setup, block_id)
                        },
                        |(setup, block_id)| black_box(setup.db.get_block_status(block_id)).unwrap(),
                    );
                }
                #[cfg(feature = "rocksdb")]
                DatabaseBackend::RocksDb => {
                    b.iter_with_setup(
                        || {
                            let setup = L2BenchSetupRocks::new();
                            let seed_data = vec![(payload_ops + 3) as u8; 1024];
                            let mut unstructured = arbitrary::Unstructured::new(&seed_data);
                            let bundle = L2BlockBundle::arbitrary(&mut unstructured)
                                .expect("Failed to generate L2BlockBundle");
                            let block_id = bundle.block().header().get_blockid();
                            setup.db.put_block_data(bundle).unwrap();
                            setup
                                .db
                                .set_block_status(block_id, BlockStatus::Valid)
                                .unwrap();
                            (setup, block_id)
                        },
                        |(setup, block_id)| black_box(setup.db.get_block_status(block_id)).unwrap(),
                    );
                }
            },
        );
    }

    group.finish();
}

// Use the macro to generate benchmarks for all available backends
alpen_benchmarks::bench_all_backends!(bench_put_block_data, bench_put_block_data_impl);
alpen_benchmarks::bench_all_backends!(bench_get_block_data, bench_get_block_data_impl);
alpen_benchmarks::bench_all_backends!(bench_set_block_status, bench_set_block_status_impl);
alpen_benchmarks::bench_all_backends!(bench_get_block_status, bench_get_block_status_impl);

criterion_group!(
    benches,
    bench_put_block_data,
    bench_get_block_data,
    bench_set_block_status,
    bench_get_block_status,
);

criterion_main!(benches);
