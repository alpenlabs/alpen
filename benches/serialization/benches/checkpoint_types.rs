//! Benchmarks comparing Borsh vs SSZ serialization for checkpoint types.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use ssz::{Decode, Encode};
use ssz_types::FixedVector;
use strata_checkpoint_types as borsh_types;
use strata_checkpoint_types_ssz as ssz_checkpoint_types;
use strata_identifiers as ids;
// Suppress unused crate warnings for dependencies used by other benchmarks
#[allow(
    unused_imports,
    clippy::allow_attributes,
    reason = "used by other benchmarks in this package"
)]
use strata_identifiers_ssz as _;
use tree_hash::TreeHash;

// ============================================================================
// BatchInfo Benchmarks
// ============================================================================

fn bench_batch_info(c: &mut Criterion) {
    let mut group = c.benchmark_group("BatchInfo");

    let l1_start = ids::L1BlockCommitment::from_height_u64(
        100,
        ids::L1BlockId::from(ids::Buf32::from([5u8; 32])),
    )
    .unwrap();
    let l1_end = ids::L1BlockCommitment::from_height_u64(
        200,
        ids::L1BlockId::from(ids::Buf32::from([6u8; 32])),
    )
    .unwrap();
    let l2_start =
        ids::L2BlockCommitment::new(1000, ids::L2BlockId::from(ids::Buf32::from([7u8; 32])));
    let l2_end =
        ids::L2BlockCommitment::new(2000, ids::L2BlockId::from(ids::Buf32::from([8u8; 32])));

    let borsh_val = borsh_types::BatchInfo::new(10, (l1_start, l1_end), (l2_start, l2_end));

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
            let decoded: borsh_types::BatchInfo = borsh::from_slice(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_val: ssz_checkpoint_types::BatchInfo = borsh_val.into();

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded = ssz_checkpoint_types::BatchInfo::from_ssz_bytes(black_box(&bytes)).unwrap();
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
// BatchTransition Benchmarks
// ============================================================================

fn bench_batch_transition(c: &mut Criterion) {
    let mut group = c.benchmark_group("BatchTransition");

    let borsh_val = borsh_types::BatchTransition {
        epoch: 5,
        chainstate_transition: borsh_types::ChainstateRootTransition {
            pre_state_root: ids::Buf32::from([3u8; 32]),
            post_state_root: ids::Buf32::from([4u8; 32]),
        },
    };

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
            let decoded: borsh_types::BatchTransition =
                borsh::from_slice(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_val: ssz_checkpoint_types::BatchTransition = borsh_val.into();

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded = ssz_checkpoint_types::BatchTransition::from_ssz_bytes(black_box(&bytes)).unwrap();
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
// CheckpointCommitment Benchmarks
// ============================================================================

fn bench_checkpoint_commitment(c: &mut Criterion) {
    let mut group = c.benchmark_group("CheckpointCommitment");

    let l1_start = ids::L1BlockCommitment::from_height_u64(
        100,
        ids::L1BlockId::from(ids::Buf32::from([5u8; 32])),
    )
    .unwrap();
    let l1_end = ids::L1BlockCommitment::from_height_u64(
        200,
        ids::L1BlockId::from(ids::Buf32::from([6u8; 32])),
    )
    .unwrap();
    let l2_start =
        ids::L2BlockCommitment::new(1000, ids::L2BlockId::from(ids::Buf32::from([7u8; 32])));
    let l2_end =
        ids::L2BlockCommitment::new(2000, ids::L2BlockId::from(ids::Buf32::from([8u8; 32])));

    let batch_info = borsh_types::BatchInfo::new(10, (l1_start, l1_end), (l2_start, l2_end));
    let batch_transition = borsh_types::BatchTransition {
        epoch: 5,
        chainstate_transition: borsh_types::ChainstateRootTransition {
            pre_state_root: ids::Buf32::from([3u8; 32]),
            post_state_root: ids::Buf32::from([4u8; 32]),
        },
    };

    // Borsh version - serialize/deserialize via Checkpoint since CheckpointCommitment is private
    let proof_data = vec![0xABu8; 1024];
    let chainstate_data = vec![0xCDu8; 10240];
    let sidecar = borsh_types::CheckpointSidecar::new(chainstate_data.clone());
    let borsh_checkpoint = borsh_types::Checkpoint::new(
        batch_info.clone(),
        batch_transition.clone(),
        proof_data.as_slice().into(),
        sidecar,
    );
    let borsh_commitment_bytes = borsh::to_vec(&borsh_checkpoint.commitment()).unwrap();

    group.throughput(Throughput::Elements(1));
    group.bench_function("borsh_serialize", |b| {
        b.iter(|| {
            let bytes = borsh::to_vec(black_box(borsh_checkpoint.commitment())).unwrap();
            black_box(bytes);
        })
    });

    group.bench_function("borsh_deserialize", |b| {
        b.iter(|| {
            let decoded: borsh_types::CheckpointCommitment =
                borsh::from_slice(black_box(&borsh_commitment_bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_batch_info: ssz_checkpoint_types::BatchInfo = batch_info.into();
    let ssz_batch_transition: ssz_checkpoint_types::BatchTransition = batch_transition.into();

    let ssz_val = ssz_checkpoint_types::CheckpointCommitment {
        batch_info: ssz_batch_info,
        transition: ssz_batch_transition,
    };

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded =
                ssz_checkpoint_types::CheckpointCommitment::from_ssz_bytes(black_box(&bytes)).unwrap();
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
// Checkpoint Benchmarks (with proof and sidecar)
// ============================================================================

fn bench_checkpoint(c: &mut Criterion) {
    let mut group = c.benchmark_group("Checkpoint");

    let l1_start = ids::L1BlockCommitment::from_height_u64(
        100,
        ids::L1BlockId::from(ids::Buf32::from([5u8; 32])),
    )
    .unwrap();
    let l1_end = ids::L1BlockCommitment::from_height_u64(
        200,
        ids::L1BlockId::from(ids::Buf32::from([6u8; 32])),
    )
    .unwrap();
    let l2_start =
        ids::L2BlockCommitment::new(1000, ids::L2BlockId::from(ids::Buf32::from([7u8; 32])));
    let l2_end =
        ids::L2BlockCommitment::new(2000, ids::L2BlockId::from(ids::Buf32::from([8u8; 32])));

    let batch_info = borsh_types::BatchInfo::new(10, (l1_start, l1_end), (l2_start, l2_end));
    let batch_transition = borsh_types::BatchTransition {
        epoch: 5,
        chainstate_transition: borsh_types::ChainstateRootTransition {
            pre_state_root: ids::Buf32::from([3u8; 32]),
            post_state_root: ids::Buf32::from([4u8; 32]),
        },
    };

    // Simulate a proof (256KB - realistic size for SP1/RISC-V ZK proofs)
    let proof_data = vec![0xABu8; 256 * 1024];

    // Simulate chainstate (100KB - realistic size for state snapshot)
    let chainstate_data = vec![0xCDu8; 100 * 1024];

    let sidecar_borsh = borsh_types::CheckpointSidecar::new(chainstate_data.clone());
    let borsh_val = borsh_types::Checkpoint::new(
        batch_info.clone(),
        batch_transition.clone(),
        proof_data.as_slice().into(),
        sidecar_borsh,
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
            let decoded: borsh_types::Checkpoint = borsh::from_slice(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_batch_info: ssz_checkpoint_types::BatchInfo = batch_info.into();
    let ssz_batch_transition: ssz_checkpoint_types::BatchTransition = batch_transition.into();

    let commitment = ssz_checkpoint_types::CheckpointCommitment {
        batch_info: ssz_batch_info,
        transition: ssz_batch_transition,
    };

    let sidecar = ssz_checkpoint_types::CheckpointSidecar {
        chainstate: chainstate_data.into(),
    };

    let ssz_val = ssz_checkpoint_types::Checkpoint {
        commitment,
        proof: proof_data.into(),
        sidecar,
    };

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded = ssz_checkpoint_types::Checkpoint::from_ssz_bytes(black_box(&bytes)).unwrap();
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
// SignedCheckpoint Benchmarks
// ============================================================================

fn bench_signed_checkpoint(c: &mut Criterion) {
    let mut group = c.benchmark_group("SignedCheckpoint");

    let l1_start = ids::L1BlockCommitment::from_height_u64(
        100,
        ids::L1BlockId::from(ids::Buf32::from([5u8; 32])),
    )
    .unwrap();
    let l1_end = ids::L1BlockCommitment::from_height_u64(
        200,
        ids::L1BlockId::from(ids::Buf32::from([6u8; 32])),
    )
    .unwrap();
    let l2_start =
        ids::L2BlockCommitment::new(1000, ids::L2BlockId::from(ids::Buf32::from([7u8; 32])));
    let l2_end =
        ids::L2BlockCommitment::new(2000, ids::L2BlockId::from(ids::Buf32::from([8u8; 32])));

    let batch_info = borsh_types::BatchInfo::new(10, (l1_start, l1_end), (l2_start, l2_end));
    let batch_transition = borsh_types::BatchTransition {
        epoch: 5,
        chainstate_transition: borsh_types::ChainstateRootTransition {
            pre_state_root: ids::Buf32::from([3u8; 32]),
            post_state_root: ids::Buf32::from([4u8; 32]),
        },
    };

    let proof_data = vec![0xABu8; 1024];
    let chainstate_data = vec![0xCDu8; 10240];

    let sidecar_borsh = borsh_types::CheckpointSidecar::new(chainstate_data.clone());
    let checkpoint_borsh = borsh_types::Checkpoint::new(
        batch_info.clone(),
        batch_transition.clone(),
        proof_data.as_slice().into(),
        sidecar_borsh,
    );

    // 64-byte signature
    let signature_bytes = ids::Buf64::from([0xEFu8; 64]);
    let borsh_val = borsh_types::SignedCheckpoint::new(checkpoint_borsh, signature_bytes);

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
            let decoded: borsh_types::SignedCheckpoint =
                borsh::from_slice(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_batch_info: ssz_checkpoint_types::BatchInfo = batch_info.into();
    let ssz_batch_transition: ssz_checkpoint_types::BatchTransition = batch_transition.into();

    let commitment = ssz_checkpoint_types::CheckpointCommitment {
        batch_info: ssz_batch_info,
        transition: ssz_batch_transition,
    };

    let sidecar = ssz_checkpoint_types::CheckpointSidecar {
        chainstate: chainstate_data.into(),
    };

    let checkpoint = ssz_checkpoint_types::Checkpoint {
        commitment,
        proof: proof_data.into(),
        sidecar,
    };

    let signature = FixedVector::from(vec![0xEFu8; 64]);

    let ssz_val = ssz_checkpoint_types::SignedCheckpoint {
        inner: checkpoint,
        signature,
    };

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded = ssz_checkpoint_types::SignedCheckpoint::from_ssz_bytes(black_box(&bytes)).unwrap();
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
// CommitmentInfo Benchmarks
// ============================================================================

fn bench_commitment_info(c: &mut Criterion) {
    let mut group = c.benchmark_group("CommitmentInfo");

    let borsh_val = borsh_types::CommitmentInfo::new(
        ids::Buf32::from([0x12u8; 32]),
        ids::Buf32::from([0x34u8; 32]),
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
            let decoded: borsh_types::CommitmentInfo =
                borsh::from_slice(black_box(&bytes)).unwrap();
            black_box(decoded);
        })
    });

    let ssz_val: ssz_checkpoint_types::CommitmentInfo = borsh_val.into();

    group.bench_function("ssz_serialize", |b| {
        b.iter(|| {
            let bytes = black_box(&ssz_val).as_ssz_bytes();
            black_box(bytes);
        })
    });

    group.bench_function("ssz_deserialize", |b| {
        let bytes = ssz_val.as_ssz_bytes();
        b.iter(|| {
            let decoded = ssz_checkpoint_types::CommitmentInfo::from_ssz_bytes(black_box(&bytes)).unwrap();
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

criterion_group!(
    benches,
    bench_batch_info,
    bench_batch_transition,
    bench_checkpoint_commitment,
    bench_checkpoint,
    bench_signed_checkpoint,
    bench_commitment_info
);
criterion_main!(benches);
