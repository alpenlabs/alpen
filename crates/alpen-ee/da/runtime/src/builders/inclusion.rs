//! Generic L1 byte-blob inclusion layer of the DA witness: walking the batch's
//! referenced L1 blocks, building Bitcoin wtxid inclusion proofs, and DA-blob
//! reassembly from the witnessed transactions.
//!
//! This layer is execution-environment agnostic — it deals only with published
//! byte blobs in L1 transactions, not with what those blobs decode into.

use alpen_ee_common::L1DaBlockRef;
use alpen_ee_da_types::{
    bitcoin_inclusion_proof, extract_da_chunks, reassemble_da_blob, wtxid_leaves,
    wtxids_root_from_txs, BitcoinMerkleProof, DaBlob, DaBlockWitness, DaTxWitness,
    L1DaBlockInclusion,
};
use bitcoin::{consensus::serialize as btc_serialize, hashes::Hash as _, Transaction};
use bitcoind_async_client::traits::Reader;
use strata_identifiers::{Buf32, WtxidsRoot};
use strata_primitives::l1::L1BlockIdBitcoinExt;

use super::DaWitnessBuildError;

/// Walks the batch's referenced L1 blocks and builds the generic byte-blob
/// inclusion witness (raw txs + wtxid Merkle proofs), returning the per-block
/// witnesses and the flat list of included transactions for blob reassembly.
///
/// This is execution-environment agnostic: it knows about L1 blocks and txs, not
/// about what the reassembled blob decodes into.
pub(crate) async fn collect_l1_inclusion_blocks(
    da_refs: &[L1DaBlockRef],
    btc: &(impl Reader + Sync),
) -> Result<(Vec<DaBlockWitness>, Vec<Transaction>), DaWitnessBuildError> {
    if da_refs.is_empty() {
        return Err(DaWitnessBuildError::EmptyDaRefs);
    }

    let mut sorted: Vec<&L1DaBlockRef> = da_refs.iter().collect();
    sorted.sort_by_key(|r| r.block.height());

    let mut blocks = Vec::with_capacity(sorted.len());
    let mut included_txs = Vec::new();
    for da_ref in sorted {
        let block_hash = da_ref.block.blkid().to_block_hash();
        let block =
            btc.get_block(&block_hash)
                .await
                .map_err(|e| DaWitnessBuildError::GetBlock {
                    block: block_hash.to_string(),
                    error: e.to_string(),
                })?;
        if block.txdata.is_empty() {
            return Err(DaWitnessBuildError::BlockHasNoTransactions(
                block_hash.to_string(),
            ));
        }
        let computed_wtxids_root = wtxids_root_from_txs(&block.txdata);
        if computed_wtxids_root != *da_ref.block.wtxids_root().as_ref() {
            let computed_wtxids_root = WtxidsRoot::from(Buf32::from(computed_wtxids_root));
            return Err(DaWitnessBuildError::WtxidsRootMismatch {
                block: block_hash.to_string(),
                expected: da_ref.block.wtxids_root().to_string(),
                computed: computed_wtxids_root.to_string(),
            });
        }

        let mut txs = Vec::with_capacity(da_ref.txns.len());
        for (txid, wtxid) in &da_ref.txns {
            let pos = block
                .txdata
                .iter()
                .position(|tx| {
                    tx.compute_txid().to_byte_array() == txid.to_byte_array()
                        && tx.compute_wtxid().to_byte_array() == wtxid.to_byte_array()
                })
                .ok_or_else(|| DaWitnessBuildError::DaTxNotFound {
                    txid: txid.to_string(),
                    wtxid: wtxid.to_string(),
                    block: block_hash.to_string(),
                })?;
            let proof = build_wtxid_inclusion_proof(&block.txdata, pos);
            let tx = block.txdata[pos].clone();
            txs.push(DaTxWitness::new(btc_serialize(&tx), proof));
            included_txs.push(tx);
        }

        blocks.push(DaBlockWitness::new(
            L1DaBlockInclusion::new(
                da_ref.block.height(),
                *da_ref.block.blkid().as_ref(),
                *da_ref.block.wtxids_root().as_ref(),
            ),
            txs,
        ));
    }

    Ok((blocks, included_txs))
}

/// Builds a wtxid-to-witness-root inclusion proof for the transaction at
/// position `idx` within `txs`.
///
/// Per BIP-141, the coinbase wtxid leaf is 32 zero bytes (see
/// [`wtxid_leaves`](alpen_ee_da_types::wtxid_leaves)).
pub(crate) fn build_wtxid_inclusion_proof(txs: &[Transaction], idx: usize) -> BitcoinMerkleProof {
    let leaves = wtxid_leaves(txs);
    let siblings = bitcoin_inclusion_proof(&leaves, idx as u32);
    BitcoinMerkleProof::new(siblings, idx as u32)
}

/// Reassembles this batch's DA blob from its included commit/reveal transactions.
pub(crate) fn reassemble_da_blob_from_txs(
    txs: &[Transaction],
) -> Result<DaBlob, DaWitnessBuildError> {
    let chunks = extract_da_chunks(txs.iter())?;
    reassemble_da_blob(&chunks).map_err(DaWitnessBuildError::Reassembly)
}

#[cfg(test)]
mod tests {
    use alpen_ee_da_types::{bitcoin_merkle_root, bitcoin_merkle_root_from_leaves};
    use bitcoin::{
        absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, TxIn,
        TxOut, Witness,
    };

    use super::*;

    fn compute_root(leaf: [u8; 32], proof: &BitcoinMerkleProof) -> [u8; 32] {
        bitcoin_merkle_root(leaf, proof.siblings(), proof.position())
    }

    fn make_dummy_tx(nonce: u8) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(vec![nonce]),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    #[test]
    fn wtxid_inclusion_proof_matches_naive_root_with_coinbase_zeroed() {
        let txs: Vec<Transaction> = (0..5).map(make_dummy_tx).collect();
        let leaves = wtxid_leaves(&txs);
        let expected_root = bitcoin_merkle_root_from_leaves(&leaves);

        for (idx, leaf) in leaves.iter().enumerate().skip(1) {
            let proof = build_wtxid_inclusion_proof(&txs, idx);
            assert_eq!(compute_root(*leaf, &proof), expected_root, "idx={idx}");
        }
    }
}
