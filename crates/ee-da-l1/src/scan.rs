//! Extracts EE DA chunked envelopes from bounded L1 block data.

use bitcoin::{
    opcodes::all::{OP_CHECKSIG, OP_RETURN},
    script::Instruction,
    secp256k1::XOnlyPublicKey,
    Block, Script, Transaction, Txid,
};
use strata_l1_envelope_fmt::{errors::EnvelopeParseError, parser::parse_envelope_payload};
use strata_l1_txfmt::MagicBytes;
use thiserror::Error;

/// Current commit marker version supported by this parser.
const SUPPORTED_COMMIT_MARKER_VERSION: u32 = 0;

/// Lightweight commit marker recognized during discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitMarker {
    version: u32,
}

impl CommitMarker {
    /// Returns the commit marker version.
    pub fn version(&self) -> u32 {
        self.version
    }
}

/// Parsed chunks for one commit/reveal envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEnvelope {
    commit_txid: Txid,
    chunks: Vec<Vec<u8>>,
}

impl ParsedEnvelope {
    pub(crate) fn new(commit_txid: Txid, chunks: Vec<Vec<u8>>) -> Self {
        Self {
            commit_txid,
            chunks,
        }
    }

    /// Returns the transaction id of the commit transaction.
    pub fn commit_txid(&self) -> Txid {
        self.commit_txid
    }

    /// Returns raw encoded DA chunks ordered by commit output vout.
    pub fn chunks(&self) -> &[Vec<u8>] {
        &self.chunks
    }

    /// Consumes the envelope and returns its ordered raw encoded DA chunks.
    pub fn into_chunks(self) -> Vec<Vec<u8>> {
        self.chunks
    }
}

/// Errors raised while discovering or parsing EE DA chunked envelopes.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("unsupported commit marker version {1} for commit {0}")]
    UnsupportedCommitMarkerVersion(Txid, u32),

    #[error("multiple commit markers for commit {0}")]
    MultipleCommitMarkers(Txid),

    #[error("invalid commit marker output for commit {0} (expected 0, got {1})")]
    InvalidCommitMarkerOutput(Txid, u32),

    #[error("missing reveal slots for commit {0}")]
    MissingRevealSlots(Txid),

    #[error("ambiguous taproot change output for commit {0} at output {1}")]
    AmbiguousTaprootChangeOutput(Txid, u32),

    #[error("missing reveal for commit {0} output {1}")]
    MissingReveal(Txid, u32),

    #[error("duplicate reveal for commit {0} output {1}")]
    DuplicateReveal(Txid, u32),

    #[error("reveal {0} spends multiple slots")]
    MultipleRevealSlotSpends(Txid),

    #[error("missing taproot leaf script in reveal {0}")]
    MissingLeafScript(Txid),

    #[error("invalid sequencer public key in reveal {0}")]
    InvalidSequencerPubkey(Txid),

    #[error("envelope payload parse failed in reveal {0}: {1}")]
    EnvelopePayloadParse(Txid, #[source] EnvelopeParseError),
}

/// Returns the commit marker for a transaction if it is a valid EE DA commit.
pub fn peek_commit_marker(tx: &Transaction, magic_bytes: MagicBytes) -> Option<CommitMarker> {
    let (vout, version) = find_commit_markers(tx, magic_bytes).next()?;
    if vout == 0 && version == SUPPORTED_COMMIT_MARKER_VERSION {
        Some(CommitMarker { version })
    } else {
        None
    }
}

/// Scans one L1 block for complete EE DA chunked envelopes.
///
/// This expects the caller to pass a confirmed block/range. Missing reveals for
/// a commit in the supplied data are hard parse errors, not a signal to wait.
pub fn scan_block(
    block: &Block,
    magic_bytes: MagicBytes,
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<Vec<ParsedEnvelope>, ScanError> {
    let mut envelopes = Vec::new();

    for tx in &block.txdata {
        if parse_commit_marker(tx, magic_bytes)?.is_none() {
            continue;
        }
        envelopes.push(parse_chunked_envelope(
            tx,
            &block.txdata,
            magic_bytes,
            sequencer_pubkey,
        )?);
    }

    Ok(envelopes)
}

/// Parses one commit transaction using candidate reveal transactions.
pub fn parse_chunked_envelope(
    commit: &Transaction,
    candidate_reveals: &[Transaction],
    magic_bytes: MagicBytes,
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<ParsedEnvelope, ScanError> {
    parse_commit_marker(commit, magic_bytes)?;

    let commit_txid = commit.compute_txid();
    let slot_count = reveal_slot_count(commit)?;
    let mut reveals_by_slot: Vec<Option<(&Transaction, usize)>> = vec![None; slot_count];

    for reveal in candidate_reveals {
        if reveal.compute_txid() == commit_txid {
            continue;
        }

        let mut matched_slot = None;
        for (input_index, input) in reveal.input.iter().enumerate() {
            if input.previous_output.txid != commit_txid {
                continue;
            }

            let vout = input.previous_output.vout;
            if vout == 0 || vout as usize > slot_count {
                continue;
            }

            if matched_slot.is_some() {
                return Err(ScanError::MultipleRevealSlotSpends(reveal.compute_txid()));
            }
            matched_slot = Some((vout as usize - 1, input_index));
        }

        let Some((slot, input_index)) = matched_slot else {
            continue;
        };

        if reveals_by_slot[slot].is_some() {
            return Err(ScanError::DuplicateReveal(commit_txid, (slot + 1) as u32));
        }
        reveals_by_slot[slot] = Some((reveal, input_index));
    }

    let mut chunks = Vec::with_capacity(slot_count);
    for (slot, reveal) in reveals_by_slot.into_iter().enumerate() {
        let (reveal, input_index) =
            reveal.ok_or(ScanError::MissingReveal(commit_txid, (slot + 1) as u32))?;
        chunks.push(extract_reveal_chunk(reveal, input_index, sequencer_pubkey)?);
    }

    Ok(ParsedEnvelope::new(commit_txid, chunks))
}

fn parse_commit_marker(
    tx: &Transaction,
    magic_bytes: MagicBytes,
) -> Result<Option<CommitMarker>, ScanError> {
    let commit_txid = tx.compute_txid();
    let mut markers = find_commit_markers(tx, magic_bytes);
    let Some((vout, version)) = markers.next() else {
        return Ok(None);
    };

    if markers.next().is_some() {
        return Err(ScanError::MultipleCommitMarkers(commit_txid));
    }

    if vout != 0 {
        return Err(ScanError::InvalidCommitMarkerOutput(
            commit_txid,
            vout as u32,
        ));
    }

    if version != SUPPORTED_COMMIT_MARKER_VERSION {
        return Err(ScanError::UnsupportedCommitMarkerVersion(
            commit_txid,
            version,
        ));
    }

    Ok(Some(CommitMarker { version }))
}

fn find_commit_markers(
    tx: &Transaction,
    magic_bytes: MagicBytes,
) -> impl Iterator<Item = (usize, u32)> + '_ {
    tx.output
        .iter()
        .enumerate()
        .filter_map(move |(vout, output)| {
            parse_commit_marker_script(output.script_pubkey.as_script(), magic_bytes)
                .map(|version| (vout, version))
        })
}

fn parse_commit_marker_script(script: &Script, magic_bytes: MagicBytes) -> Option<u32> {
    let mut instructions = script.instructions_minimal();
    match instructions.next()? {
        Ok(Instruction::Op(OP_RETURN)) => {}
        _ => return None,
    }

    let payload = match instructions.next()? {
        Ok(Instruction::PushBytes(bytes)) if bytes.len() == 8 => bytes,
        _ => return None,
    };

    if instructions.next().is_some() {
        return None;
    }

    let payload = payload.as_bytes();
    if &payload[..4] != magic_bytes.as_bytes() {
        return None;
    }

    let mut version = [0u8; 4];
    version.copy_from_slice(&payload[4..]);
    Some(u32::from_be_bytes(version))
}

fn reveal_slot_count(commit: &Transaction) -> Result<usize, ScanError> {
    let commit_txid = commit.compute_txid();
    let mut count = 0usize;
    let mut reveal_run_closed = false;

    for (vout, output) in commit.output.iter().enumerate().skip(1) {
        if output.script_pubkey.is_p2tr() {
            if reveal_run_closed {
                return Err(ScanError::AmbiguousTaprootChangeOutput(
                    commit_txid,
                    vout as u32,
                ));
            }
            count += 1;
        } else if count > 0 {
            reveal_run_closed = true;
        }
    }

    if count == 0 {
        return Err(ScanError::MissingRevealSlots(commit_txid));
    }

    Ok(count)
}

fn extract_reveal_chunk(
    reveal: &Transaction,
    input_index: usize,
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<Vec<u8>, ScanError> {
    let reveal_txid = reveal.compute_txid();
    let leaf_script = reveal
        .input
        .get(input_index)
        .and_then(|input| input.witness.taproot_leaf_script())
        .ok_or(ScanError::MissingLeafScript(reveal_txid))?
        .script
        .to_owned();

    if !script_uses_sequencer_pubkey(&leaf_script, sequencer_pubkey) {
        return Err(ScanError::InvalidSequencerPubkey(reveal_txid));
    }

    parse_envelope_payload(&leaf_script)
        .map_err(|source| ScanError::EnvelopePayloadParse(reveal_txid, source))
}

fn script_uses_sequencer_pubkey(script: &Script, sequencer_pubkey: XOnlyPublicKey) -> bool {
    let mut instructions = script.instructions_minimal();

    match instructions.next() {
        Some(Ok(Instruction::PushBytes(bytes)))
            if bytes.as_bytes() == sequencer_pubkey.serialize() => {}
        _ => return false,
    }

    matches!(instructions.next(), Some(Ok(Instruction::Op(OP_CHECKSIG))))
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        secp256k1::{Keypair, Secp256k1, SecretKey},
        OutPoint, ScriptBuf, Sequence, TxIn, Witness,
    };
    use proptest::prelude::*;

    use super::*;
    use crate::test_utils::{
        build_block_with_txs, build_commit_marker_script, build_commit_tx, build_reveal_tx,
        chunk_body_strategy, magic_bytes_strategy,
    };

    fn test_magic() -> MagicBytes {
        "ALPN".parse().expect("valid ASCII magic")
    }

    fn test_key(seed: u8) -> XOnlyPublicKey {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[seed; 32]).expect("valid secret key");
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        XOnlyPublicKey::from_keypair(&keypair).0
    }

    #[test]
    fn peek_commit_marker_accepts_exact_marker_at_output_zero() {
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let marker = peek_commit_marker(&commit, test_magic()).expect("marker");
        assert_eq!(marker.version(), 0);
    }

    proptest! {
        #[test]
        fn peek_commit_marker_rejects_wrong_magic(wrong_magic in magic_bytes_strategy()) {
            prop_assume!(wrong_magic != *b"ALPN");
            let commit = build_commit_tx(MagicBytes::new(wrong_magic), 0, 1, false);
            prop_assert_eq!(peek_commit_marker(&commit, test_magic()), None);
        }

        #[test]
        fn parse_chunked_envelope_orders_chunks_by_commit_vout(
            chunk0 in chunk_body_strategy(32),
            chunk1 in chunk_body_strategy(32),
        ) {
            let sequencer_pubkey = test_key(7);
            let commit = build_commit_tx(test_magic(), 0, 2, false);
            let commit_txid = commit.compute_txid();
            let reveal1 = build_reveal_tx(commit_txid, 2, sequencer_pubkey, &chunk1);
            let reveal0 = build_reveal_tx(commit_txid, 1, sequencer_pubkey, &chunk0);

            let parsed = parse_chunked_envelope(
                &commit,
                &[reveal1, reveal0],
                test_magic(),
                sequencer_pubkey,
            )
            .expect("envelope parses");

            prop_assert_eq!(parsed.commit_txid(), commit_txid);
            prop_assert_eq!(parsed.chunks(), vec![chunk0, chunk1]);
        }

        #[test]
        fn parse_chunked_envelope_rejects_missing_reveal(chunk in chunk_body_strategy(32)) {
            let sequencer_pubkey = test_key(7);
            let commit = build_commit_tx(test_magic(), 0, 2, false);
            let reveal0 = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, &chunk);

            let err = parse_chunked_envelope(&commit, &[reveal0], test_magic(), sequencer_pubkey)
                .expect_err("missing reveal must fail");

            prop_assert!(
                matches!(err, ScanError::MissingReveal(_, 2)),
                "expected missing reveal error, got {err:?}"
            );
        }

        #[test]
        fn parse_chunked_envelope_rejects_duplicate_reveal(
            chunk0 in chunk_body_strategy(32),
            chunk1 in chunk_body_strategy(32),
        ) {
            let sequencer_pubkey = test_key(7);
            let commit = build_commit_tx(test_magic(), 0, 1, false);
            let commit_txid = commit.compute_txid();
            let reveal0 = build_reveal_tx(commit_txid, 1, sequencer_pubkey, &chunk0);
            let reveal1 = build_reveal_tx(commit_txid, 1, sequencer_pubkey, &chunk1);

            let err = parse_chunked_envelope(
                &commit,
                &[reveal0, reveal1],
                test_magic(),
                sequencer_pubkey,
            )
            .expect_err("duplicate reveal must fail");

            prop_assert!(
                matches!(err, ScanError::DuplicateReveal(_, 1)),
                "expected duplicate reveal error, got {err:?}"
            );
        }
    }

    #[test]
    fn parse_chunked_envelope_rejects_wrong_sequencer_pubkey() {
        let sequencer_pubkey = test_key(7);
        let other_pubkey = test_key(8);
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, other_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("wrong sequencer pubkey must fail");

        assert!(matches!(err, ScanError::InvalidSequencerPubkey(_)));
    }

    #[test]
    fn parse_chunked_envelope_rejects_unsupported_commit_version() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 1, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("unsupported version must fail");

        assert!(matches!(
            err,
            ScanError::UnsupportedCommitMarkerVersion(_, 1)
        ));
    }

    #[test]
    fn parse_chunked_envelope_rejects_multiple_commit_markers() {
        let sequencer_pubkey = test_key(7);
        let mut commit = build_commit_tx(test_magic(), 0, 1, false);
        commit.output.push(bitcoin::TxOut {
            value: bitcoin::Amount::from_sat(0),
            script_pubkey: build_commit_marker_script(test_magic(), 0),
        });
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("multiple commit markers must fail");

        assert!(matches!(err, ScanError::MultipleCommitMarkers(_)));
    }

    #[test]
    fn parse_chunked_envelope_rejects_commit_marker_after_output_zero() {
        let sequencer_pubkey = test_key(7);
        let mut commit = build_commit_tx(test_magic(), 0, 1, false);
        commit.output.swap(0, 1);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("misplaced commit marker must fail");

        assert!(matches!(err, ScanError::InvalidCommitMarkerOutput(_, 1)));
    }

    #[test]
    fn parse_chunked_envelope_rejects_missing_reveal_slots() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 0, false);

        let err = parse_chunked_envelope(&commit, &[], test_magic(), sequencer_pubkey)
            .expect_err("missing reveal slots must fail");

        assert!(matches!(err, ScanError::MissingRevealSlots(_)));
    }

    #[test]
    fn parse_chunked_envelope_rejects_reveal_spending_multiple_slots() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 2, false);
        let mut reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        reveal.input.push(TxIn {
            previous_output: OutPoint {
                txid: commit.compute_txid(),
                vout: 2,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        });

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("one reveal spending multiple slots must fail");

        assert!(matches!(err, ScanError::MultipleRevealSlotSpends(_)));
    }

    #[test]
    fn parse_chunked_envelope_rejects_p2tr_change_ambiguity() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, true);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("ambiguous P2TR change must fail");

        assert!(matches!(err, ScanError::AmbiguousTaprootChangeOutput(_, _)));
    }

    #[test]
    fn scan_block_extracts_complete_envelope() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let block = build_block_with_txs(vec![commit.clone(), reveal]);

        let envelopes = scan_block(&block, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit.compute_txid());
        assert_eq!(envelopes[0].chunks(), vec![b"chunk".to_vec()]);
    }

    #[test]
    fn build_commit_marker_script_uses_one_payload_push() {
        let script = build_commit_marker_script(test_magic(), 0);
        assert_eq!(
            parse_commit_marker_script(script.as_script(), test_magic()),
            Some(0)
        );
    }
}
