//! Host-side helpers for assembling the DA witness consumed by the EE acct proof.

use std::collections::{BTreeMap, BTreeSet};

use alloy_primitives::{keccak256, B256};
use alpen_ee_common::{reassemble_da_blob, DaBlob};
use alpen_reth_statediff::{AccountChange, BatchStateDiff};
use bitcoin::{hashes::Hash as _, opcodes::all::OP_RETURN, script::Instruction, Transaction, Txid};
use strata_crypto::hash::sha256d;
use strata_ee_acct_runtime::{BitcoinMerkleProof, DaBytecodeWitness};
use strata_l1_envelope_fmt::parser::parse_envelope_payload;

/// Builds a wtxid-to-witness-root inclusion proof for the transaction at
/// position `idx` within `txs`.
///
/// Per BIP-141, the coinbase wtxid leaf is 32 zero bytes.
pub(super) fn build_wtxid_inclusion_proof(txs: &[Transaction], idx: usize) -> BitcoinMerkleProof {
    let leaves: Vec<[u8; 32]> = txs
        .iter()
        .enumerate()
        .map(|(i, tx)| {
            if i == 0 {
                [0u8; 32]
            } else {
                tx.compute_wtxid().to_byte_array()
            }
        })
        .collect();
    build_inclusion_proof(&leaves, idx as u32)
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
    let leaves: Vec<[u8; 32]> = txs
        .iter()
        .enumerate()
        .map(|(i, tx)| {
            if i == 0 {
                [0u8; 32]
            } else {
                tx.compute_wtxid().to_byte_array()
            }
        })
        .collect();
    merkle_root(&leaves)
}

/// Reassembles this batch's DA blob from its included commit/reveal transactions.
pub(super) fn reassemble_da_blob_from_txs(txs: &[Transaction]) -> eyre::Result<DaBlob> {
    let chunks = extract_da_chunks(txs)?;
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

fn extract_da_chunks(txs: &[Transaction]) -> eyre::Result<Vec<Vec<u8>>> {
    let mut commit: Option<&Transaction> = None;
    let mut non_commit_txs = Vec::new();

    for tx in txs {
        if commit_marker_payload(tx)?.is_some() {
            if commit.replace(tx).is_some() {
                return Err(eyre::eyre!("multiple DA commit transactions in witness"));
            }
        } else {
            non_commit_txs.push(tx);
        }
    }

    let commit = commit.ok_or_else(|| eyre::eyre!("missing DA commit transaction in witness"))?;
    let commit_txid = commit.compute_txid();
    let last_reveal_vout = last_commit_reveal_vout(commit);

    let mut chunks_by_vout = BTreeMap::new();
    for tx in non_commit_txs {
        let (vout, chunk) = extract_reveal_chunk(tx, commit_txid)?;
        if vout > last_reveal_vout {
            return Err(eyre::eyre!("unexpected DA reveal for commit output {vout}"));
        }
        if chunks_by_vout.insert(vout, chunk).is_some() {
            return Err(eyre::eyre!("duplicate DA reveal for commit output {vout}"));
        }
    }

    for expected_vout in 1..=last_reveal_vout {
        if !chunks_by_vout.contains_key(&expected_vout) {
            return Err(eyre::eyre!(
                "missing DA reveal for commit output {expected_vout}"
            ));
        }
    }

    Ok(chunks_by_vout.into_values().collect())
}

fn last_commit_reveal_vout(commit: &Transaction) -> u32 {
    commit
        .output
        .iter()
        .enumerate()
        .skip(1)
        .take_while(|(_, output)| output.script_pubkey.is_p2tr())
        .map(|(idx, _)| idx as u32)
        .last()
        .unwrap_or(0)
}

fn commit_marker_payload(tx: &Transaction) -> eyre::Result<Option<[u8; 8]>> {
    let Some(first_output) = tx.output.first() else {
        return Ok(None);
    };
    let mut instructions = first_output.script_pubkey.instructions();
    let Some(Ok(Instruction::Op(OP_RETURN))) = instructions.next() else {
        return Ok(None);
    };
    let Some(Ok(Instruction::PushBytes(push))) = instructions.next() else {
        return Err(eyre::eyre!("malformed DA commit marker"));
    };
    if instructions.next().is_some() || push.as_bytes().len() != 8 {
        return Err(eyre::eyre!("malformed DA commit marker"));
    }

    let mut payload = [0u8; 8];
    payload.copy_from_slice(push.as_bytes());
    Ok(Some(payload))
}

fn extract_reveal_chunk(reveal: &Transaction, commit_txid: Txid) -> eyre::Result<(u32, Vec<u8>)> {
    let input = reveal
        .input
        .first()
        .ok_or_else(|| eyre::eyre!("DA reveal tx has no inputs"))?;
    if input.previous_output.txid != commit_txid {
        return Err(eyre::eyre!("DA reveal tx does not spend the DA commit tx"));
    }
    let vout = input.previous_output.vout;
    if vout == 0 {
        return Err(eyre::eyre!("DA reveal spends commit output 0"));
    }

    let leaf = input
        .witness
        .taproot_leaf_script()
        .ok_or_else(|| eyre::eyre!("DA reveal tx witness has no tapscript leaf"))?;
    let script = leaf.script.into();
    let chunk = parse_envelope_payload(&script)?;

    Ok((vout, chunk))
}

fn build_inclusion_proof(leaves: &[[u8; 32]], idx: u32) -> BitcoinMerkleProof {
    assert!(
        (idx as usize) < leaves.len(),
        "idx {idx} out of bounds for {} leaves",
        leaves.len()
    );

    let mut cur_level = leaves.to_vec();
    let mut cur_idx = idx;
    let depth = (usize::BITS - cur_level.len().leading_zeros()) as usize;
    let mut siblings = Vec::with_capacity(depth);

    while cur_level.len() > 1 {
        if cur_level.len() % 2 == 1 {
            cur_level.push(*cur_level.last().expect("non-empty level"));
        }

        siblings.push(cur_level[(cur_idx ^ 1) as usize]);

        cur_level = cur_level
            .chunks(2)
            .map(|pair| {
                let mut preimage = [0u8; 64];
                preimage[..32].copy_from_slice(&pair[0]);
                preimage[32..].copy_from_slice(&pair[1]);
                *sha256d(&preimage).as_ref()
            })
            .collect();
        cur_idx >>= 1;
    }

    BitcoinMerkleProof::new(siblings, idx)
}

fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    let mut cur_level = leaves.to_vec();
    while cur_level.len() > 1 {
        if cur_level.len() % 2 == 1 {
            cur_level.push(*cur_level.last().expect("non-empty level"));
        }

        cur_level = cur_level
            .chunks(2)
            .map(|pair| {
                let mut preimage = [0u8; 64];
                preimage[..32].copy_from_slice(&pair[0]);
                preimage[32..].copy_from_slice(&pair[1]);
                *sha256d(&preimage).as_ref()
            })
            .collect();
    }

    cur_level[0]
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};
    use alpen_reth_statediff::{AccountDiff, BatchStateDiff};
    use bitcoin::{
        absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence,
        Transaction, TxIn, TxOut, Witness,
    };

    use super::*;

    fn hash_pair(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
        let mut preimage = [0u8; 64];
        preimage[..32].copy_from_slice(&left);
        preimage[32..].copy_from_slice(&right);
        *sha256d(&preimage).as_ref()
    }

    fn compute_root(leaf: [u8; 32], proof: &BitcoinMerkleProof) -> [u8; 32] {
        let mut cur = leaf;
        let mut pos = proof.position();
        for sibling in proof.siblings() {
            cur = if pos & 1 == 0 {
                hash_pair(cur, *sibling)
            } else {
                hash_pair(*sibling, cur)
            };
            pos >>= 1;
        }
        cur
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

    fn naive_merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
        let mut cur = leaves.to_vec();
        while cur.len() > 1 {
            if cur.len() % 2 == 1 {
                cur.push(*cur.last().expect("non-empty level"));
            }
            cur = cur
                .chunks(2)
                .map(|pair| hash_pair(pair[0], pair[1]))
                .collect();
        }
        cur[0]
    }

    #[test]
    fn wtxid_inclusion_proof_matches_naive_root_with_coinbase_zeroed() {
        let txs: Vec<Transaction> = (0..5).map(make_dummy_tx).collect();
        let leaves: Vec<[u8; 32]> = txs
            .iter()
            .enumerate()
            .map(|(idx, tx)| {
                if idx == 0 {
                    [0u8; 32]
                } else {
                    tx.compute_wtxid().to_byte_array()
                }
            })
            .collect();
        let expected_root = naive_merkle_root(&leaves);

        for (idx, leaf) in leaves.iter().enumerate().skip(1) {
            let proof = build_wtxid_inclusion_proof(&txs, idx);
            assert_eq!(compute_root(*leaf, &proof), expected_root, "idx={idx}");
        }
    }

    #[test]
    fn compute_wtxids_root_matches_naive_root_with_coinbase_zeroed() {
        let txs: Vec<Transaction> = (0..5).map(make_dummy_tx).collect();
        let leaves: Vec<[u8; 32]> = txs
            .iter()
            .enumerate()
            .map(|(idx, tx)| {
                if idx == 0 {
                    [0u8; 32]
                } else {
                    tx.compute_wtxid().to_byte_array()
                }
            })
            .collect();

        assert_eq!(compute_wtxids_root(&txs), naive_merkle_root(&leaves));
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
            evm_header: alpen_ee_common::EvmHeaderSummary {
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
