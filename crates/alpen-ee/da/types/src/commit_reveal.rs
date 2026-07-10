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
    #[error("DA reveal spends multiple outputs of the DA commit tx")]
    RevealMultipleCommitSpends,
    #[error("DA commit has no reveal slots")]
    MissingRevealSlots,
    #[error("duplicate DA reveal for commit output {0}")]
    DuplicateReveal(u32),
    #[error("unexpected DA reveal for commit output {0}")]
    UnexpectedReveal(u32),
    #[error("missing DA reveal for commit output {0}")]
    MissingReveal(u32),
    #[error("ambiguous P2TR change output at commit output {0}")]
    AmbiguousTaprootChangeOutput(u32),
    #[error("DA reveal envelope parse failed: {0}")]
    RevealEnvelope(#[from] EnvelopeParseError),
}

/// Extracts and orders the DA payload chunks carried by a single DA blob.
///
/// `txs` must be exactly the transactions that make up one DA blob: its single
/// commit transaction plus the reveal transactions spending that commit. This
/// is *not* a block scanner that finds arbitrary blobs, handling exactly one
/// blob is the required behaviour, so more than one commit is a
/// [`DaParseError::MultipleCommits`], and every reveal must spend the one
/// commit (and not its OP_RETURN marker output).
///
/// Returns the chunks in commit-output (vout) order. The caller checks the
/// commit marker's magic bytes / version against its own expected values; this
/// parser is format-agnostic.
pub fn extract_da_chunks<'a>(
    txs: impl Iterator<Item = &'a Transaction>,
) -> Result<Vec<Vec<u8>>, DaParseError> {
    let mut commit: Option<&Transaction> = None;
    let mut non_commit_txs = Vec::new();

    for tx in txs {
        if read_commit_marker_payload(tx)?.is_some() {
            if commit.replace(tx).is_some() {
                return Err(DaParseError::MultipleCommits);
            }
        } else {
            non_commit_txs.push(tx);
        }
    }

    let commit = commit.ok_or(DaParseError::MissingCommit)?;
    let commit_txid = commit.compute_txid();
    let last_reveal_vout =
        last_commit_reveal_vout(commit)?.ok_or(DaParseError::MissingRevealSlots)?;

    let mut chunks_by_vout = BTreeMap::new();
    for tx in non_commit_txs {
        let (vout, chunk) = extract_reveal_chunk(tx, commit_txid, last_reveal_vout)?;
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

/// Vout of the last contiguous P2TR reveal output on a commit transaction, or
/// `None` if it has no reveal outputs.
///
/// Scans outputs starting at vout 1 (vout 0 is the OP_RETURN marker) and
/// returns the last vout of the contiguous P2TR run.
pub fn last_commit_reveal_vout(commit: &Transaction) -> Result<Option<u32>, DaParseError> {
    let mut last_reveal_vout = None;
    let mut reveal_run_closed = false;

    for (idx, output) in commit.output.iter().enumerate().skip(1) {
        if output.script_pubkey.is_p2tr() {
            if reveal_run_closed {
                return Err(DaParseError::AmbiguousTaprootChangeOutput(idx as u32));
            }
            last_reveal_vout = Some(idx as u32);
        } else {
            reveal_run_closed = true;
        }
    }

    Ok(last_reveal_vout)
}

/// Returns whether `vout` is inside a commit transaction's reveal-slot range.
///
/// Returns `false` when the commit has no reveal slots.
pub fn is_reveal_slot(vout: u32, last_reveal_vout: Option<u32>) -> bool {
    matches!(last_reveal_vout, Some(last) if (1..=last).contains(&vout))
}

/// Reads the 8-byte OP_RETURN marker payload (`magic || version`) from a
/// commit transaction's vout-0 output, if present.
///
/// Callers scanning arbitrary L1 traffic should pre-filter by magic to avoid
/// spurious [`DaParseError::MalformedCommitMarker`] errors on unrelated
/// OP_RETURN protocols.
///
/// Returns:
/// - `Ok(Some([u8; 8]))` if the first output is a well-formed OP_RETURN with exactly 8 pushed bytes
///   (`magic_bytes || version_be_u32`).
/// - `Ok(None)` if the first output is missing or not an OP_RETURN (i.e. this isn't a commit
///   transaction).
/// - `Err(MalformedCommitMarker)` if the OP_RETURN exists but has the wrong shape (extra ops, wrong
///   push size, etc.).
pub fn read_commit_marker_payload(tx: &Transaction) -> Result<Option<[u8; 8]>, DaParseError> {
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
fn extract_reveal_chunk(
    reveal: &Transaction,
    commit_txid: Txid,
    last_reveal_vout: u32,
) -> Result<(u32, Vec<u8>), DaParseError> {
    if reveal.input.is_empty() {
        return Err(DaParseError::RevealMissingInputs);
    }

    let mut matching_input = None;
    let mut unexpected_vout = None;
    for input in &reveal.input {
        if input.previous_output.txid != commit_txid {
            continue;
        }
        let vout = input.previous_output.vout;
        if vout == 0 {
            return Err(DaParseError::RevealSpendsMarker);
        }
        if !is_reveal_slot(vout, Some(last_reveal_vout)) {
            unexpected_vout.get_or_insert(vout);
            continue;
        }
        if matching_input.replace(input).is_some() {
            return Err(DaParseError::RevealMultipleCommitSpends);
        }
    }

    let input = matching_input.ok_or_else(|| {
        unexpected_vout.map_or(
            DaParseError::RevealWrongCommit,
            DaParseError::UnexpectedReveal,
        )
    })?;
    let vout = input.previous_output.vout;

    let leaf = input
        .witness
        .taproot_leaf_script()
        .ok_or(DaParseError::RevealMissingLeafScript)?;
    let script = leaf.script.into();
    let chunk = parse_envelope_payload(&script)?;

    Ok((vout, chunk))
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime,
        hashes::Hash,
        opcodes::all::OP_RETURN,
        script::Builder,
        secp256k1::{Keypair, Parity, Secp256k1, SecretKey, XOnlyPublicKey},
        taproot::{ControlBlock, LeafVersion, TapNodeHash, TaprootMerkleBranch},
        transaction::Version,
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    };
    use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;

    use super::*;

    fn txid(seed: u8) -> Txid {
        Txid::from_byte_array([seed; 32])
    }

    fn test_key(seed: u8) -> XOnlyPublicKey {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[seed; 32]).expect("valid secret key");
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        XOnlyPublicKey::from_keypair(&keypair).0
    }

    fn test_control_block(internal_key: XOnlyPublicKey) -> ControlBlock {
        let branch: [TapNodeHash; 0] = [];

        ControlBlock {
            leaf_version: LeafVersion::TapScript,
            output_key_parity: Parity::Even,
            internal_key,
            merkle_branch: TaprootMerkleBranch::from(branch),
        }
    }

    fn reveal_script(chunk: &[u8]) -> ScriptBuf {
        let sequencer_pubkey = test_key(7);
        EnvelopeScriptBuilder::with_pubkey(&sequencer_pubkey.serialize())
            .expect("pubkey accepted")
            .add_envelope(chunk)
            .expect("envelope payload accepted")
            .build_without_min_check()
            .expect("reveal script build succeeds")
    }

    fn reveal_input(commit_txid: Txid, vout: u32, chunk: Option<&[u8]>) -> TxIn {
        let mut witness = Witness::new();
        if let Some(chunk) = chunk {
            let sequencer_pubkey = test_key(7);
            witness.push([1u8; 64]);
            witness.push(reveal_script(chunk));
            witness.push(test_control_block(sequencer_pubkey).serialize());
        }

        TxIn {
            previous_output: OutPoint {
                txid: commit_txid,
                vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        }
    }

    fn reveal_tx(inputs: Vec<TxIn>) -> Transaction {
        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: inputs,
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: Builder::new().push_opcode(OP_RETURN).into_script(),
            }],
        }
    }

    #[test]
    fn extract_reveal_chunk_ignores_out_of_range_commit_input() {
        let commit_txid = txid(1);
        let tx = reveal_tx(vec![
            reveal_input(commit_txid, 1, Some(b"chunk")),
            reveal_input(commit_txid, 2, None),
        ]);

        let (vout, chunk) = extract_reveal_chunk(&tx, commit_txid, 1).expect("valid reveal");

        assert_eq!(vout, 1);
        assert_eq!(chunk, b"chunk");
    }

    #[test]
    fn extract_reveal_chunk_rejects_only_out_of_range_commit_input() {
        let commit_txid = txid(1);
        let tx = reveal_tx(vec![reveal_input(commit_txid, 2, None)]);

        let error = extract_reveal_chunk(&tx, commit_txid, 1).expect_err("unexpected reveal");

        assert!(matches!(error, DaParseError::UnexpectedReveal(2)));
    }

    #[test]
    fn extract_reveal_chunk_rejects_multiple_slot_inputs() {
        let commit_txid = txid(1);
        let tx = reveal_tx(vec![
            reveal_input(commit_txid, 1, None),
            reveal_input(commit_txid, 2, None),
        ]);

        let error = extract_reveal_chunk(&tx, commit_txid, 2).expect_err("multiple slot inputs");

        assert!(matches!(error, DaParseError::RevealMultipleCommitSpends));
    }

    #[test]
    fn extract_reveal_chunk_rejects_marker_spend() {
        let commit_txid = txid(1);
        let tx = reveal_tx(vec![reveal_input(commit_txid, 0, None)]);

        let error = extract_reveal_chunk(&tx, commit_txid, 1).expect_err("marker spend");

        assert!(matches!(error, DaParseError::RevealSpendsMarker));
    }

    #[test]
    fn extract_reveal_chunk_rejects_wrong_commit() {
        let tx = reveal_tx(vec![reveal_input(txid(2), 1, None)]);

        let error = extract_reveal_chunk(&tx, txid(1), 1).expect_err("wrong commit");

        assert!(matches!(error, DaParseError::RevealWrongCommit));
    }
}
