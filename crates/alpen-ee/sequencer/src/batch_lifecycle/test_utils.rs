//! Shared test utilities for batch_lifecycle tests.

use alpen_ee_common::{
    Batch, BatchId, BatchStatus, BatchStorage, InMemoryStorage, L1DaBlockRef, ProofId,
};
use bitcoin::{absolute, hashes::Hash as _, BlockHash, Txid, Wtxid};
use strata_acct_types::Hash;
use strata_identifiers::{L1BlockCommitment, L1BlockId};

/// Helper to create a test hash from a single byte.
pub(crate) fn test_hash(n: u8) -> Hash {
    let mut buf = [0u8; 32];
    buf[0] = 1; // ensure ZERO hash is not created
    buf[31] = n;
    Hash::from(buf)
}

/// Helper to create a BatchId for testing.
pub(crate) fn make_batch_id(prev_n: u8, last_n: u8) -> BatchId {
    BatchId::from_parts(test_hash(prev_n), test_hash(last_n))
}

/// Helper to create a Batch for testing.
pub(crate) fn make_batch(idx: u64, prev_n: u8, last_n: u8) -> Batch {
    Batch::new(
        idx,
        test_hash(prev_n),
        test_hash(last_n),
        last_n as u64,
        vec![],
    )
    .expect("valid batch")
}

/// Helper to create a genesis batch for testing.
pub(crate) fn make_genesis_batch(n: u8) -> Batch {
    Batch::new_genesis_batch(test_hash(n), n as u64).expect("valid genesis batch")
}

/// Helper to create a test Txid.
pub(crate) fn test_txid(n: u8) -> Txid {
    let mut buf = [0u8; 32];
    buf[31] = n;
    Txid::from_byte_array(buf)
}

/// Helper to create a test Wtxid.
pub(crate) fn test_wtxid(n: u8) -> Wtxid {
    let mut buf = [0u8; 32];
    buf[31] = n;
    Wtxid::from_byte_array(buf)
}

/// Helper to create test L1DaBlockRef.
pub(crate) fn make_da_ref(block_n: u8, txn_n: u8) -> L1DaBlockRef {
    let block_hash = BlockHash::from_byte_array([block_n; 32]);
    let height = absolute::Height::from_consensus(block_n as u32).expect("valid height");
    let blkid = L1BlockId::from(block_hash);
    L1DaBlockRef {
        block: L1BlockCommitment::new(height, blkid),
        txns: vec![(test_txid(txn_n), test_wtxid(txn_n))],
    }
}

/// Helper to create a ProofId for testing.
pub(crate) fn test_proof_id(n: u8) -> ProofId {
    ProofId::new(test_hash(n).into())
}

/// Simplified batch status for test helper (auto-generates dummy DA/proof data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, clippy::allow_attributes, reason = "test helper")]
pub(crate) enum TestBatchStatus {
    Sealed,
    DaPending,
    DaComplete,
    ProofPending,
    ProofReady,
}

/// Fill storage with a genesis batch followed by batches with the specified statuses.
///
/// - Genesis batch (idx=0, status=Genesis) is always created first
/// - First status in list -> batch idx=1, second -> idx=2, etc.
/// - Batches are linked: each batch's prev_block = previous batch's last_block
/// - DA/proof data is auto-generated using batch index
pub(crate) async fn fill_storage(
    storage: &impl BatchStorage,
    statuses: &[TestBatchStatus],
) -> Vec<Batch> {
    // Save genesis batch (idx=0, last_block=test_hash(0))
    let genesis = make_genesis_batch(0);
    storage.save_genesis_batch(genesis.clone()).await.unwrap();
    let mut batches = vec![genesis];

    for (i, test_status) in statuses.iter().enumerate() {
        let idx = (i + 1) as u64;
        let prev_n = i as u8; // links to previous batch's last_block
        let last_n = (i + 1) as u8; // unique last_block for this batch

        let batch = make_batch(idx, prev_n, last_n);
        storage.save_next_batch(batch.clone()).await.unwrap();

        // Convert TestBatchStatus to BatchStatus with dummy data
        let status = match test_status {
            TestBatchStatus::Sealed => BatchStatus::Sealed,
            TestBatchStatus::DaPending => BatchStatus::DaPending { envelope_idx: 0 },
            TestBatchStatus::DaComplete => BatchStatus::DaComplete {
                da: vec![make_da_ref(last_n, last_n)],
            },
            TestBatchStatus::ProofPending => BatchStatus::ProofPending {
                da: vec![make_da_ref(last_n, last_n)],
            },
            TestBatchStatus::ProofReady => BatchStatus::ProofReady {
                da: vec![make_da_ref(last_n, last_n)],
                proof: test_proof_id(last_n),
            },
        };

        // Update status if not Sealed (save_next_batch creates with Sealed)
        if !matches!(status, BatchStatus::Sealed) {
            storage
                .update_batch_status(batch.id(), status)
                .await
                .unwrap();
        }

        batches.push(batch);
    }

    batches
}

pub(crate) fn read_batch_statuses(storage: impl AsRef<InMemoryStorage>) -> Vec<TestBatchStatus> {
    storage
        .as_ref()
        .batches
        .read()
        .unwrap()
        .iter()
        .filter_map(|(_, (_, batch_status))| match batch_status {
            BatchStatus::Genesis => None,
            BatchStatus::Sealed => Some(TestBatchStatus::Sealed),
            BatchStatus::DaPending { .. } => Some(TestBatchStatus::DaPending),
            BatchStatus::DaComplete { .. } => Some(TestBatchStatus::DaComplete),
            BatchStatus::ProofPending { .. } => Some(TestBatchStatus::ProofPending),
            BatchStatus::ProofReady { .. } => Some(TestBatchStatus::ProofReady),
        })
        .collect()
}
