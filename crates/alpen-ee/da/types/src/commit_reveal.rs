//! Bitcoin commit/reveal parsing for chunked DA envelope inscriptions.
//!
//! Shared between the witness builder (host) and the proof verifier (guest),
//! both of which need to extract DA chunks from a set of L1 transactions in a
//! deterministic order.

use std::collections::BTreeMap;

use bitcoin::{opcodes::all::OP_RETURN, script::Instruction, Transaction, Txid};
use strata_l1_envelope_fmt::{errors::EnvelopeParseError, parser::parse_envelope_payload};
use thiserror::Error;

/// Errors raised while parsing DA commit/reveal transactions.
#[derive(Debug, Error)]
pub enum DaParseError {
    #[error("DA witness has no commit transaction")]
    MissingCommit,
    #[error("DA witness has multiple commit transactions")]
    MultipleCommits,
    #[error("DA commit OP_RETURN marker is malformed")]
    MalformedCommitMarker,
    #[error("DA reveal tx has no inputs")]
    RevealMissingInputs,
    #[error("DA reveal tx witness has no tapscript leaf")]
    RevealMissingLeafScript,
    #[error("DA reveal tx does not spend the DA commit tx")]
    RevealWrongCommit,
    #[error("DA reveal spends commit output 0, which is the OP_RETURN marker")]
    RevealSpendsMarker,
    #[error("duplicate DA reveal for commit output {0}")]
    DuplicateReveal(u32),
    #[error("unexpected DA reveal for commit output {0}")]
    UnexpectedReveal(u32),
    #[error("missing DA reveal for commit output {0}")]
    MissingReveal(u32),
    #[error("DA reveal envelope parse failed: {0}")]
    RevealEnvelope(#[from] EnvelopeParseError),
}

/// Extracts and orders DA payload chunks from a set of L1 transactions.
///
/// Returns the chunks in commit-output order. Caller is responsible for
/// checking the commit marker's magic bytes / version against its own expected
/// values; this parser is format-agnostic.
pub fn extract_da_chunks<'a>(
    txs: impl Iterator<Item = &'a Transaction>,
) -> Result<Vec<Vec<u8>>, DaParseError> {
    let mut commit: Option<&Transaction> = None;
    let mut non_commit_txs = Vec::new();

    for tx in txs {
        if commit_marker_payload(tx)?.is_some() {
            if commit.replace(tx).is_some() {
                return Err(DaParseError::MultipleCommits);
            }
        } else {
            non_commit_txs.push(tx);
        }
    }

    let commit = commit.ok_or(DaParseError::MissingCommit)?;
    let commit_txid = commit.compute_txid();
    let last_reveal_vout = last_commit_reveal_vout(commit);

    let mut chunks_by_vout = BTreeMap::new();
    for tx in non_commit_txs {
        let (vout, chunk) = extract_reveal_chunk(tx, commit_txid)?;
        if vout > last_reveal_vout {
            return Err(DaParseError::UnexpectedReveal(vout));
        }
        if chunks_by_vout.insert(vout, chunk).is_some() {
            return Err(DaParseError::DuplicateReveal(vout));
        }
    }

    for expected_vout in 1..=last_reveal_vout {
        if !chunks_by_vout.contains_key(&expected_vout) {
            return Err(DaParseError::MissingReveal(expected_vout));
        }
    }

    Ok(chunks_by_vout.into_values().collect())
}

/// Vout of the last P2TR reveal output on a commit transaction.
///
/// Scans outputs starting at vout 1 (vout 0 is the OP_RETURN marker) and
/// returns the last contiguous P2TR vout. Returns 0 if there are no reveal
/// outputs.
pub fn last_commit_reveal_vout(commit: &Transaction) -> u32 {
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

/// Reads the 8-byte OP_RETURN marker payload (`magic || version`) from a
/// commit transaction's vout-0 output, if present.
///
/// Returns:
/// - `Ok(Some([u8; 8]))` if the first output is a well-formed OP_RETURN with exactly 8 pushed bytes
///   (`magic_bytes || version_be_u32`).
/// - `Ok(None)` if the first output is missing or not an OP_RETURN (i.e. this isn't a commit
///   transaction).
/// - `Err(MalformedCommitMarker)` if the OP_RETURN exists but has the wrong shape (extra ops, wrong
///   push size, etc.).
pub fn commit_marker_payload(tx: &Transaction) -> Result<Option<[u8; 8]>, DaParseError> {
    let Some(first_output) = tx.output.first() else {
        return Ok(None);
    };
    let mut instructions = first_output.script_pubkey.instructions();
    let Some(Ok(Instruction::Op(OP_RETURN))) = instructions.next() else {
        return Ok(None);
    };
    let Some(Ok(Instruction::PushBytes(push))) = instructions.next() else {
        return Err(DaParseError::MalformedCommitMarker);
    };
    if instructions.next().is_some() || push.as_bytes().len() != 8 {
        return Err(DaParseError::MalformedCommitMarker);
    }

    let mut payload = [0u8; 8];
    payload.copy_from_slice(push.as_bytes());
    Ok(Some(payload))
}

/// Extracts the `(vout, chunk_bytes)` carried by a single DA reveal tx.
///
/// The reveal must spend a non-zero output of the named commit tx; the chunk
/// payload is parsed from the tapscript leaf via [`parse_envelope_payload`].
pub fn extract_reveal_chunk(
    reveal: &Transaction,
    commit_txid: Txid,
) -> Result<(u32, Vec<u8>), DaParseError> {
    let input = reveal
        .input
        .first()
        .ok_or(DaParseError::RevealMissingInputs)?;
    if input.previous_output.txid != commit_txid {
        return Err(DaParseError::RevealWrongCommit);
    }
    let vout = input.previous_output.vout;
    if vout == 0 {
        return Err(DaParseError::RevealSpendsMarker);
    }

    let leaf = input
        .witness
        .taproot_leaf_script()
        .ok_or(DaParseError::RevealMissingLeafScript)?;
    let script = leaf.script.into();
    let chunk = parse_envelope_payload(&script)?;

    Ok((vout, chunk))
}
