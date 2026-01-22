//! Shared test utilities for batch_lifecycle tests.

use alpen_ee_common::{Batch, BatchId, L1DaBlockRef, ProofId};
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
