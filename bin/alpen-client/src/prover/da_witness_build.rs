//! Host-side helpers for assembling the DA witness consumed by the EE acct proof.

use std::collections::BTreeSet;

use alloy_primitives::{keccak256, B256};
use alpen_ee_da_types::{
    bitcoin_inclusion_proof, bitcoin_merkle_root_from_leaves, extract_da_chunks,
    reassemble_da_blob, DaBlob,
};
use alpen_reth_statediff::{AccountChange, BatchStateDiff};
use bitcoin::{hashes::Hash as _, Transaction};
use strata_ee_acct_runtime::{BitcoinMerkleProof, DaBytecodeWitness};

/// Builds a wtxid-to-witness-root inclusion proof for the transaction at
/// position `idx` within `txs`.
///
/// Per BIP-141, the coinbase wtxid leaf is 32 zero bytes.
pub(super) fn build_wtxid_inclusion_proof(txs: &[Transaction], idx: usize) -> BitcoinMerkleProof {
    let leaves = wtxid_leaves(txs);
    let siblings = bitcoin_inclusion_proof(&leaves, idx as u32);
    BitcoinMerkleProof::new(siblings, idx as u32)
}

/// Computes the BIP-141 witness transaction Merkle root for a block's tx list.
///
/// Per BIP-141, the coinbase wtxid leaf is 32 zero bytes.
pub(super) fn compute_wtxids_root(txs: &[Transaction]) -> [u8; 32] {
    assert!(
        !txs.is_empty(),
        "wtxids root requires at least the coinbase tx"
    );

    // NOTE: DA reveal txs are witness spends, and Alpen DA commit tx funding is
    // expected to spend post-SegWit inputs. Under that writer invariant, every
    // DA block referenced here has witness data and this root matches the L1
    // block ref root committed by ASM. If a future writer allows legacy-input
    // commit funding, commit-only blocks must mirror ASM's txid-root fallback.
    let leaves = wtxid_leaves(txs);
    bitcoin_merkle_root_from_leaves(&leaves)
}

/// Returns wtxid leaves for `txs`, zeroing the coinbase leaf per BIP-141.
fn wtxid_leaves(txs: &[Transaction]) -> Vec<[u8; 32]> {
    txs.iter()
        .enumerate()
        .map(|(i, tx)| {
            if i == 0 {
                [0u8; 32]
            } else {
                tx.compute_wtxid().to_byte_array()
            }
        })
        .collect()
}

/// Reassembles this batch's DA blob from its included commit/reveal transactions.
pub(super) fn reassemble_da_blob_from_txs(txs: &[Transaction]) -> eyre::Result<DaBlob> {
    let chunks =
        extract_da_chunks(txs.iter()).map_err(|e| eyre::eyre!("extract DA chunks: {e}"))?;
    reassemble_da_blob(&chunks).map_err(|e| eyre::eyre!("reassemble DA blob: {e}"))
}

/// Returns account code hashes referenced by the blob but absent from the current blob bytecodes.
///
/// These are the hashes affected by DA bytecode dedupe: the account diff still
/// advertises a `code_hash`, but the current L1 blob no longer carries the
/// matching bytecode bytes.
pub(super) fn deduped_account_code_hashes(blob: &DaBlob) -> Vec<B256> {
    let empty_code_hash = keccak256([]);
    let mut missing = BTreeSet::new();

    for change in blob.state_diff.accounts.values() {
        let account_diff = match change {
            AccountChange::Created(diff) | AccountChange::Updated(diff) => diff,
            AccountChange::Deleted => continue,
        };
        let Some(code_hash) = account_diff.code_hash.new_value().map(|hash| hash.0) else {
            continue;
        };
        if code_hash == empty_code_hash
            || blob.state_diff.deployed_bytecodes.contains_key(&code_hash)
        {
            continue;
        }
        missing.insert(code_hash);
    }

    missing.into_iter().collect()
}

/// Builds private bytecode witness entries from the unfiltered batch state diff.
///
/// The DA blob passed to the guest has already gone through the publication
/// filter, so bytecodes published by earlier batches can be missing from
/// `blob.state_diff.deployed_bytecodes`. The unfiltered per-block state diffs
/// are local host data from the same executed batch before that filter ran, and
/// they still carry deployment bytecodes even when the current DA blob omitted
/// them. Using this source avoids depending on the accessed-state cache, which
/// only stores bytecode loaded through `code_by_hash` and can miss a contract
/// that was deployed but never executed/read again.
///
/// This is still a local reconstruction witness. The guest re-hashes these
/// bytes to prove they match the account diff's `code_hash`, but the proper
/// future solution is an authenticated prior-publication proof for omitted
/// bytecodes.
pub(super) fn known_bytecodes_from_unfiltered_diff(
    blob: &DaBlob,
    unfiltered_state_diff: &BatchStateDiff,
) -> (Vec<DaBytecodeWitness>, Vec<B256>) {
    let mut known_bytecodes = Vec::new();
    let mut unresolved = Vec::new();

    for code_hash in deduped_account_code_hashes(blob) {
        match unfiltered_state_diff.deployed_bytecodes.get(&code_hash) {
            Some(bytecode) => {
                known_bytecodes.push(DaBytecodeWitness::new(code_hash.0, bytecode.to_vec()));
            }
            None => unresolved.push(code_hash),
        }
    }

    (known_bytecodes, unresolved)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};
    use alpen_ee_da_types::{bitcoin_merkle_root, EvmHeaderSummary};
    use alpen_reth_statediff::{AccountDiff, BatchStateDiff};
    use bitcoin::{
        absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence,
        Transaction, TxIn, TxOut, Witness,
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

    #[test]
    fn compute_wtxids_root_matches_naive_root_with_coinbase_zeroed() {
        let txs: Vec<Transaction> = (0..5).map(make_dummy_tx).collect();
        let leaves = wtxid_leaves(&txs);
        assert_eq!(
            compute_wtxids_root(&txs),
            bitcoin_merkle_root_from_leaves(&leaves)
        );
    }

    #[test]
    fn known_bytecodes_from_unfiltered_diff_recovers_deduped_deployment_bytecode() {
        let bytecode = Bytes::from_static(&[0x60, 0x80, 0x60, 0x40, 0x52]);
        let code_hash = keccak256(bytecode.as_ref());
        let address = Address::from([0x11; 20]);

        let mut filtered_diff = BatchStateDiff::new();
        filtered_diff.accounts.insert(
            address,
            AccountChange::Created(AccountDiff::new_created(U256::ZERO, 1, code_hash)),
        );

        let mut unfiltered_diff = filtered_diff.clone();
        unfiltered_diff
            .deployed_bytecodes
            .insert(code_hash, bytecode.clone());

        let blob = DaBlob {
            update_seq_no: 7,
            evm_header: EvmHeaderSummary {
                block_num: 10,
                timestamp: 1_700_000_000,
                base_fee: 100,
                gas_used: 21_000,
                gas_limit: 30_000_000,
            },
            state_diff: filtered_diff,
        };

        let (known, unresolved) = known_bytecodes_from_unfiltered_diff(&blob, &unfiltered_diff);

        assert!(unresolved.is_empty());
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].code_hash(), &code_hash.0);
        assert_eq!(known[0].bytecode(), bytecode.as_ref());
    }
}
