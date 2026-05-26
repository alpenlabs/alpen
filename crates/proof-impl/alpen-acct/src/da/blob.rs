//! DA blob extraction and metadata checks.

use std::collections::{BTreeMap, BTreeSet};

use alloy_primitives::keccak256;
use alpen_reth_statediff::{
    AccountChange, BatchStateDiff, apply_batch_state_diff_to_ethereum_state,
};
use bitcoin::{Transaction, Txid, opcodes::all::OP_RETURN, script::Instruction};
use revm_primitives::{B256, KECCAK_EMPTY};
use strata_codec::{Codec, CodecError, decode_buf_exact};
use strata_ee_acct_runtime::ArchivedDaBytecodeWitness;
use strata_ee_chain_types::ChunkTransition;
use strata_evm_ee::EvmPartialState;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;
use strata_snark_acct_types::UpdateProofPubParams;

use super::{
    constants::{DA_BLOB_VERSION, EE_DA_MAGIC_BYTES},
    error::DaVerificationError,
};

/// Compact summary of the last EVM block header in a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Codec)]
pub struct EvmHeaderSummary {
    /// Block number of the last EVM block in this batch.
    pub block_num: u64,
    /// Unix timestamp of the last EVM block.
    pub timestamp: u64,
    /// Base fee per gas of the last EVM block.
    pub base_fee: u64,
    /// Total gas consumed by the last EVM block.
    pub gas_used: u64,
    /// Gas limit of the last EVM block.
    pub gas_limit: u64,
}

/// DA blob containing batch metadata and state diff.
#[derive(Debug, Clone, Codec)]
pub struct DaBlob {
    /// Monotonic EE account update sequence number for this blob.
    pub update_seq_no: u64,
    /// EVM header context of the last block in this batch.
    pub evm_header: EvmHeaderSummary,
    /// Aggregated state diff for the batch.
    pub state_diff: BatchStateDiff,
}

/// Reassembles a [`DaBlob`] from raw chunk payloads.
pub(super) fn reassemble_da_blob(chunks: &[Vec<u8>]) -> Result<DaBlob, CodecError> {
    if chunks.is_empty() {
        return Err(CodecError::MalformedField("no DA chunks provided"));
    }

    let blob: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
    decode_buf_exact(&blob)
}

/// Extracts and orders DA payload chunks from included commit/reveal txs.
pub(super) fn extract_da_chunks<'a>(
    txs: impl Iterator<Item = &'a Transaction>,
) -> Result<Vec<Vec<u8>>, DaVerificationError> {
    let mut commit: Option<&Transaction> = None;
    let mut non_commit_txs = Vec::new();

    for tx in txs {
        if commit_marker_payload(tx)?.is_some() {
            if commit.replace(tx).is_some() {
                return Err(DaVerificationError::MultipleCommits);
            }
        } else {
            non_commit_txs.push(tx);
        }
    }

    let commit = commit.ok_or(DaVerificationError::MissingCommit)?;
    verify_commit_marker(commit)?;
    let commit_txid = commit.compute_txid();
    let last_reveal_vout = last_commit_reveal_vout(commit);

    let mut chunks_by_vout = BTreeMap::new();
    for tx in non_commit_txs {
        let (vout, chunk) = extract_reveal_chunk(tx, commit_txid)?;
        if vout > last_reveal_vout {
            return Err(DaVerificationError::UnexpectedReveal(vout));
        }
        if chunks_by_vout.insert(vout, chunk).is_some() {
            return Err(DaVerificationError::DuplicateReveal(vout));
        }
    }

    for expected_vout in 1..=last_reveal_vout {
        if !chunks_by_vout.contains_key(&expected_vout) {
            return Err(DaVerificationError::MissingReveal(expected_vout));
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

fn verify_commit_marker(commit: &Transaction) -> Result<(), DaVerificationError> {
    let payload = commit_marker_payload(commit)?.ok_or(DaVerificationError::MissingCommit)?;
    let actual_magic: [u8; 4] = payload[..4]
        .try_into()
        .expect("payload length checked below");
    if actual_magic != EE_DA_MAGIC_BYTES {
        return Err(DaVerificationError::CommitMagicMismatch {
            expected: EE_DA_MAGIC_BYTES,
            actual: actual_magic,
        });
    }

    let version_bytes: [u8; 4] = payload[4..]
        .try_into()
        .expect("payload length checked below");
    let actual_version = u32::from_be_bytes(version_bytes);
    if actual_version != DA_BLOB_VERSION {
        return Err(DaVerificationError::CommitVersionMismatch {
            expected: DA_BLOB_VERSION,
            actual: actual_version,
        });
    }

    Ok(())
}

fn commit_marker_payload(commit: &Transaction) -> Result<Option<[u8; 8]>, DaVerificationError> {
    let Some(first_output) = commit.output.first() else {
        return Ok(None);
    };
    let mut instructions = first_output.script_pubkey.instructions();
    let Some(Ok(Instruction::Op(OP_RETURN))) = instructions.next() else {
        return Ok(None);
    };
    let Some(Ok(Instruction::PushBytes(push))) = instructions.next() else {
        return Err(DaVerificationError::MalformedCommitMarker);
    };
    if instructions.next().is_some() || push.as_bytes().len() != 8 {
        return Err(DaVerificationError::MalformedCommitMarker);
    }

    let mut payload = [0u8; 8];
    payload.copy_from_slice(push.as_bytes());
    Ok(Some(payload))
}

fn extract_reveal_chunk(
    reveal: &Transaction,
    commit_txid: Txid,
) -> Result<(u32, Vec<u8>), DaVerificationError> {
    let input = reveal
        .input
        .first()
        .ok_or(DaVerificationError::RevealMissingInputs)?;
    if input.previous_output.txid != commit_txid {
        return Err(DaVerificationError::RevealWrongCommit);
    }
    let vout = input.previous_output.vout;
    if vout == 0 {
        return Err(DaVerificationError::RevealSpendsMarker);
    }

    let leaf = input
        .witness
        .taproot_leaf_script()
        .ok_or(DaVerificationError::RevealMissingLeafScript)?;
    let script = leaf.script.into();
    let chunk = parse_envelope_payload(&script)?;

    Ok((vout, chunk))
}

/// Verifies DA blob metadata against public params and the last chunk pubvals.
pub(super) fn verify_da_blob_metadata(
    blob: &DaBlob,
    last_chunk: &ChunkTransition,
    pub_params: &UpdateProofPubParams,
    known_bytecodes: &[ArchivedDaBytecodeWitness],
) -> Result<(), DaVerificationError> {
    let expected_seq_no = pub_params.seq_no();
    if blob.update_seq_no != expected_seq_no {
        return Err(DaVerificationError::UpdateSeqNoMismatch {
            expected: expected_seq_no,
            actual: blob.update_seq_no,
        });
    }

    let expected_header: EvmHeaderSummary =
        decode_buf_exact(last_chunk.tip_exec_header_summary().opaque_bytes())
            .map_err(DaVerificationError::ExecHeaderSummaryDecode)?;
    if blob.evm_header != expected_header {
        return Err(DaVerificationError::EvmHeaderMismatch {
            expected: expected_header,
            actual: blob.evm_header,
        });
    }

    verify_deployed_bytecodes(&blob.state_diff, known_bytecodes)
}

fn verify_deployed_bytecodes(
    diff: &BatchStateDiff,
    known_bytecodes: &[ArchivedDaBytecodeWitness],
) -> Result<(), DaVerificationError> {
    let mut available_code_hashes = BTreeSet::new();

    for (code_hash, bytecode) in &diff.deployed_bytecodes {
        let computed = keccak256(bytecode.as_ref());
        if computed != *code_hash {
            return Err(DaVerificationError::DeployedBytecodeHashMismatch {
                expected: b256_bytes(*code_hash),
                computed: b256_bytes(computed),
            });
        }
        available_code_hashes.insert(*code_hash);
    }

    // NOTE: known bytecodes are private reconstruction witness data for bytecodes
    // deduped from the current L1 blob. The guest accepts them only after
    // recomputing the EVM code hash, which proves identity for the account diff
    // but not prior L1 publication. A future protocol-level solution should
    // verify membership in an authenticated published-bytecode set, or verify
    // explicit inclusion in the earlier DA blob that first carried the bytes.
    for bytecode in known_bytecodes {
        let code_hash = B256::from(*bytecode.code_hash());
        let computed = keccak256(bytecode.bytecode());
        if computed != code_hash {
            return Err(DaVerificationError::KnownBytecodeHashMismatch {
                expected: b256_bytes(code_hash),
                computed: b256_bytes(computed),
            });
        }
        available_code_hashes.insert(code_hash);
    }

    for change in diff.accounts.values() {
        let account_diff = match change {
            AccountChange::Created(diff) | AccountChange::Updated(diff) => diff,
            AccountChange::Deleted => continue,
        };
        let Some(code_hash) = account_diff.code_hash.new_value().map(|hash| hash.0) else {
            continue;
        };
        if code_hash == KECCAK_EMPTY {
            continue;
        }
        if !available_code_hashes.contains(&code_hash) {
            return Err(DaVerificationError::MissingDeployedBytecode(b256_bytes(
                code_hash,
            )));
        }
    }

    Ok(())
}

fn b256_bytes(value: B256) -> [u8; 32] {
    value.0
}

/// Applies the DA blob state diff to the partial pre-state witness.
///
/// The input pre-state must match the EE account state's previous execution
/// root, and the post-apply root must match the last chunk transition's public
/// `tip_state_root`.
pub(super) fn verify_state_diff_against_chunks(
    raw_pre_state: &[u8],
    expected_pre_state_root: [u8; 32],
    blob: &DaBlob,
    last_chunk: &ChunkTransition,
) -> Result<(), DaVerificationError> {
    let mut pre_state: EvmPartialState =
        decode_buf_exact(raw_pre_state).map_err(DaVerificationError::PartialPreStateDecode)?;
    let actual_pre_state_root = pre_state.ethereum_state().state_root().0;
    if actual_pre_state_root != expected_pre_state_root {
        return Err(DaVerificationError::PartialPreStateRootMismatch {
            expected: expected_pre_state_root,
            actual: actual_pre_state_root,
        });
    }

    apply_batch_state_diff_to_ethereum_state(pre_state.ethereum_state_mut(), &blob.state_diff)?;

    let computed: [u8; 32] = pre_state.ethereum_state().state_root().0;
    let expected: [u8; 32] = last_chunk.tip_state_root().0;
    if computed != expected {
        return Err(DaVerificationError::StateRootMismatch { computed, expected });
    }
    Ok(())
}
