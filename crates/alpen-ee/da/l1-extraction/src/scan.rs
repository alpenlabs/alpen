//! Extracts EE DA chunked envelopes from bounded L1 block data.

use std::{collections::BTreeMap, iter};

use alpen_ee_da_types::{
    extract_da_chunks, is_reveal_slot, last_commit_reveal_vout, read_commit_marker_payload,
    DaParseError, DA_BLOB_VERSION,
};
use bitcoin::{
    opcodes::all::{OP_CHECKSIG, OP_RETURN},
    script::Instruction,
    secp256k1::XOnlyPublicKey,
    Script, Transaction, Txid,
};
use strata_l1_txfmt::MagicBytes;
use thiserror::Error;

use crate::fetch::L1BlockData;

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
    #[error("malformed commit marker in transaction {txid}: {source}")]
    MalformedCommitMarker {
        txid: Txid,
        #[source]
        source: DaParseError,
    },

    #[error("missing commit marker in transaction {txid}")]
    MissingCommitMarker { txid: Txid },

    #[error("unsupported commit marker version {version} for commit {commit_txid}")]
    UnsupportedCommitMarkerVersion { commit_txid: Txid, version: u32 },

    #[error("duplicate commit transaction id {commit_txid}")]
    DuplicateCommitTxid { commit_txid: Txid },

    #[error("reveal {reveal_txid} spends slots from multiple commits")]
    MultipleRevealCommitSpends { reveal_txid: Txid },

    #[error("missing taproot leaf script in reveal {reveal_txid}")]
    MissingLeafScript { reveal_txid: Txid },

    #[error("invalid sequencer public key in reveal {reveal_txid}")]
    InvalidSequencerPubkey { reveal_txid: Txid },

    #[error("DA envelope parse failed for commit {commit_txid}: {source}")]
    EnvelopeParse {
        commit_txid: Txid,
        #[source]
        source: DaParseError,
    },
}

#[derive(Debug)]
struct CommitCandidate {
    tx: Transaction,
    txid: Txid,
    last_reveal_vout: Option<u32>,
}

#[derive(Clone, Debug)]
struct RevealCandidate {
    tx: Transaction,
    txid: Txid,
}

/// Incrementally scans a bounded L1 range for EE DA commit/reveal envelopes.
///
/// The scanner keeps only discovered commit and reveal candidate transactions;
/// it does not retain every fetched Bitcoin block.
#[derive(Debug)]
pub struct L1RangeScanner {
    magic_bytes: MagicBytes,
    sequencer_pubkey: XOnlyPublicKey,
    commits_by_txid: BTreeMap<Txid, CommitCandidate>,
    reveals_by_commit: BTreeMap<Txid, BTreeMap<Txid, RevealCandidate>>,
}

impl L1RangeScanner {
    /// Creates an empty scanner for one confirmed L1 range.
    pub fn new(magic_bytes: MagicBytes, sequencer_pubkey: XOnlyPublicKey) -> Self {
        Self {
            magic_bytes,
            sequencer_pubkey,
            commits_by_txid: BTreeMap::new(),
            reveals_by_commit: BTreeMap::new(),
        }
    }

    /// Ingests one fetched L1 block.
    pub fn ingest_block(&mut self, block: &bitcoin::Block) -> Result<(), ScanError> {
        for tx in &block.txdata {
            self.ingest_transaction(tx)?;
        }
        Ok(())
    }

    /// Finalizes all discovered commits into parsed envelopes.
    pub fn finish(self) -> Result<Vec<ParsedEnvelope>, ScanError> {
        self.commits_by_txid
            .values()
            .map(|commit| self.finalize_commit(commit))
            .collect()
    }

    fn ingest_transaction(&mut self, tx: &Transaction) -> Result<(), ScanError> {
        let txid = tx.compute_txid();

        if parse_commit_marker(tx, txid, self.magic_bytes)?.is_some() {
            let last_reveal_vout =
                last_commit_reveal_vout(tx).map_err(|source| ScanError::EnvelopeParse {
                    commit_txid: txid,
                    source,
                })?;
            let candidate = CommitCandidate {
                tx: tx.clone(),
                txid,
                last_reveal_vout,
            };
            if self.commits_by_txid.insert(txid, candidate).is_some() {
                return Err(ScanError::DuplicateCommitTxid { commit_txid: txid });
            }
        }

        // Bitcoin blocks order parents before children, so same-block reveals
        // cannot appear before the commit output they spend.
        let mut spent_commit_txid = None;
        for input in &tx.input {
            let candidate_commit_txid = input.previous_output.txid;
            let Some(commit) = self.commits_by_txid.get(&candidate_commit_txid) else {
                continue;
            };
            if !is_reveal_slot(input.previous_output.vout, commit.last_reveal_vout) {
                continue;
            }

            match spent_commit_txid {
                None => spent_commit_txid = Some(candidate_commit_txid),
                Some(existing) if existing != candidate_commit_txid => {
                    return Err(ScanError::MultipleRevealCommitSpends { reveal_txid: txid });
                }
                Some(_) => {}
            }
        }

        if let Some(commit_txid) = spent_commit_txid {
            self.reveals_by_commit
                .entry(commit_txid)
                .or_default()
                .entry(txid)
                .or_insert_with(|| RevealCandidate {
                    tx: tx.clone(),
                    txid,
                });
        }

        Ok(())
    }

    fn finalize_commit(&self, commit: &CommitCandidate) -> Result<ParsedEnvelope, ScanError> {
        let reveals = self
            .reveals_by_commit
            .get(&commit.txid)
            .map(|reveals| reveals.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        parse_one_envelope(&commit.tx, commit.txid, &reveals, self.sequencer_pubkey)
    }
}

/// Returns the commit marker for a transaction if it is a valid EE DA commit.
pub fn peek_commit_marker(
    tx: &Transaction,
    magic_bytes: MagicBytes,
) -> Result<Option<CommitMarker>, ScanError> {
    parse_commit_marker(tx, tx.compute_txid(), magic_bytes)
}

/// Scans a confirmed L1 range for complete EE DA chunked envelopes.
///
/// Prefer [`L1RangeScanner`] for production fetch paths so fetched blocks can be
/// dropped after ingestion. This helper is intended for tests and callers that
/// already hold the full range in memory.
///
/// Commit and reveal transactions may be finalized in different blocks inside
/// the supplied range. Missing reveals are reported only after the whole range
/// is scanned.
pub fn scan_blocks(
    blocks: &[L1BlockData],
    magic_bytes: MagicBytes,
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<Vec<ParsedEnvelope>, ScanError> {
    let mut scanner = L1RangeScanner::new(magic_bytes, sequencer_pubkey);
    for block in blocks {
        scanner.ingest_block(block.block())?;
    }
    scanner.finish()
}

/// Parses one explicit commit transaction using candidate reveal transactions.
///
/// This is a single-envelope helper. Production L1 range extraction should use
/// [`L1RangeScanner`] so commits and reveals can be discovered across fetched
/// blocks without retaining the full block range.
pub fn parse_chunked_envelope(
    commit: &Transaction,
    candidate_reveals: &[Transaction],
    magic_bytes: MagicBytes,
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<ParsedEnvelope, ScanError> {
    let commit_txid = commit.compute_txid();
    parse_commit_marker_strict(commit, commit_txid, magic_bytes)?
        .ok_or(ScanError::MissingCommitMarker { txid: commit_txid })?;

    let reveals = candidate_reveals
        .iter()
        .map(|tx| RevealCandidate {
            tx: tx.clone(),
            txid: tx.compute_txid(),
        })
        .collect::<Vec<_>>();

    parse_one_envelope(commit, commit_txid, &reveals, sequencer_pubkey)
}

fn parse_commit_marker(
    tx: &Transaction,
    txid: Txid,
    magic_bytes: MagicBytes,
) -> Result<Option<CommitMarker>, ScanError> {
    let Some(payload) = read_exact_commit_marker_payload_for_magic(tx, magic_bytes) else {
        return Ok(None);
    };

    parse_commit_marker_payload(txid, payload).map(Some)
}

fn parse_commit_marker_strict(
    tx: &Transaction,
    txid: Txid,
    magic_bytes: MagicBytes,
) -> Result<Option<CommitMarker>, ScanError> {
    let Some(payload) = read_commit_marker_payload_for_magic_strict(tx, magic_bytes)
        .map_err(|source| ScanError::MalformedCommitMarker { txid, source })?
    else {
        return Ok(None);
    };

    parse_commit_marker_payload(txid, payload).map(Some)
}

fn parse_commit_marker_payload(txid: Txid, payload: [u8; 8]) -> Result<CommitMarker, ScanError> {
    let mut version = [0u8; 4];
    version.copy_from_slice(&payload[4..]);
    let version = u32::from_be_bytes(version);
    if version != DA_BLOB_VERSION {
        return Err(ScanError::UnsupportedCommitMarkerVersion {
            commit_txid: txid,
            version,
        });
    }

    Ok(CommitMarker { version })
}

fn read_exact_commit_marker_payload_for_magic(
    tx: &Transaction,
    magic_bytes: MagicBytes,
) -> Option<[u8; 8]> {
    let first_output = tx.output.first()?;
    let mut instructions = first_output.script_pubkey.instructions();
    let Some(Ok(Instruction::Op(OP_RETURN))) = instructions.next() else {
        return None;
    };
    let Some(Ok(Instruction::PushBytes(push))) = instructions.next() else {
        return None;
    };
    if instructions.next().is_some() || push.as_bytes().len() != 8 {
        return None;
    }
    if !push.as_bytes().starts_with(magic_bytes.as_bytes()) {
        return None;
    }

    let mut payload = [0u8; 8];
    payload.copy_from_slice(push.as_bytes());
    Some(payload)
}

fn read_commit_marker_payload_for_magic_strict(
    tx: &Transaction,
    magic_bytes: MagicBytes,
) -> Result<Option<[u8; 8]>, DaParseError> {
    if !first_op_return_push_starts_with_magic(tx, magic_bytes) {
        return Ok(None);
    }

    read_commit_marker_payload(tx)
}

fn first_op_return_push_starts_with_magic(tx: &Transaction, magic_bytes: MagicBytes) -> bool {
    let Some(first_output) = tx.output.first() else {
        return false;
    };
    let mut instructions = first_output.script_pubkey.instructions();
    let Some(Ok(Instruction::Op(OP_RETURN))) = instructions.next() else {
        return false;
    };
    let Some(Ok(Instruction::PushBytes(push))) = instructions.next() else {
        return false;
    };

    push.as_bytes().starts_with(magic_bytes.as_bytes())
}

fn parse_one_envelope(
    commit: &Transaction,
    commit_txid: Txid,
    reveals: &[RevealCandidate],
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<ParsedEnvelope, ScanError> {
    let last_reveal_vout =
        last_commit_reveal_vout(commit).map_err(|source| ScanError::EnvelopeParse {
            commit_txid,
            source,
        })?;

    for reveal in reveals {
        authenticate_reveal(
            &reveal.tx,
            reveal.txid,
            commit_txid,
            last_reveal_vout,
            sequencer_pubkey,
        )?;
    }

    let chunks = extract_da_chunks(iter::once(commit).chain(reveals.iter().map(|r| &r.tx)))
        .map_err(|source| ScanError::EnvelopeParse {
            commit_txid,
            source,
        })?;

    Ok(ParsedEnvelope::new(commit_txid, chunks))
}

fn authenticate_reveal(
    reveal: &Transaction,
    reveal_txid: Txid,
    commit_txid: Txid,
    last_reveal_vout: Option<u32>,
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<(), ScanError> {
    for input in &reveal.input {
        if input.previous_output.txid != commit_txid {
            continue;
        }
        if !is_reveal_slot(input.previous_output.vout, last_reveal_vout) {
            continue;
        }

        let leaf_script = input
            .witness
            .taproot_leaf_script()
            .ok_or(ScanError::MissingLeafScript { reveal_txid })?
            .script
            .to_owned();

        if !script_uses_sequencer_pubkey(&leaf_script, sequencer_pubkey) {
            return Err(ScanError::InvalidSequencerPubkey { reveal_txid });
        }
    }

    Ok(())
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
        absolute::LockTime,
        block::{Header, Version},
        hashes::Hash,
        pow::CompactTarget,
        script::Builder,
        secp256k1::{Keypair, Secp256k1, SecretKey},
        transaction::Version as TxVersion,
        Address, Amount, FeeRate, Network, OutPoint, ScriptBuf, Sequence, TxIn, TxMerkleNode,
        TxOut, Txid, Witness,
    };
    use bitcoind_async_client::corepc_types::model::ListUnspentItem;
    use proptest::prelude::*;
    use strata_btcio::writer::{
        builder::EnvelopeConfig, chunked_envelope::build_chunked_envelope_txs,
    };
    use strata_l1_txfmt::{ParseConfig, TagDataRef};

    use super::*;
    use crate::test_utils::{
        build_block_with_txs, build_commit_tx, build_reveal_tx, chunk_body_strategy,
        magic_bytes_strategy,
    };

    fn test_magic() -> MagicBytes {
        "ALPN".parse().expect("valid ASCII magic")
    }

    fn test_key(seed: u8) -> XOnlyPublicKey {
        XOnlyPublicKey::from_keypair(&test_keypair(seed)).0
    }

    fn test_keypair(seed: u8) -> Keypair {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[seed; 32]).expect("valid secret key");
        Keypair::from_secret_key(&secp, &secret_key)
    }

    fn test_change_address() -> Address {
        let script = ScriptBuf::new_p2wpkh(&bitcoin::WPubkeyHash::all_zeros());
        Address::from_script(&script, Network::Regtest).expect("valid P2WPKH address")
    }

    fn test_utxo(address: &Address) -> ListUnspentItem {
        ListUnspentItem {
            txid: "4cfbec13cf1510545f285cceceb6229bd7b6a918a8f6eba1dbee64d26226a3b7"
                .parse::<Txid>()
                .expect("valid txid"),
            vout: 0,
            address: address.as_unchecked().clone(),
            script_pubkey: ScriptBuf::new(),
            amount: Amount::from_btc(100.0).expect("valid amount"),
            confirmations: 100,
            spendable: true,
            solvable: true,
            label: String::new(),
            safe: true,
            redeem_script: None,
            descriptor: None,
            parent_descriptors: None,
        }
    }

    fn block_data(height: u32, txs: Vec<Transaction>) -> L1BlockData {
        L1BlockData::new(
            height,
            bitcoin::BlockHash::from_byte_array([height as u8; 32]),
            build_block_with_txs(txs),
        )
    }

    fn malformed_protocol_marker_script() -> ScriptBuf {
        let mut payload = [0u8; 8];
        payload[..4].copy_from_slice(test_magic().as_bytes());

        Builder::new()
            .push_opcode(OP_RETURN)
            .push_slice(payload)
            .push_opcode(OP_RETURN)
            .into_script()
    }

    fn unrelated_malformed_op_return_tx() -> Transaction {
        Transaction {
            version: TxVersion(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: Builder::new()
                    .push_opcode(OP_RETURN)
                    .push_slice(*b"NOTE")
                    .push_opcode(OP_RETURN)
                    .into_script(),
            }],
        }
    }

    fn malformed_same_magic_op_return_tx() -> Transaction {
        Transaction {
            version: TxVersion(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: Builder::new()
                    .push_opcode(OP_RETURN)
                    .push_slice(*b"ALPNnot-da")
                    .into_script(),
            }],
        }
    }

    fn sps50_checkpoint_like_tx() -> Transaction {
        let tag = TagDataRef::new(1, 1, &[]).expect("valid checkpoint-like tag");
        let script_pubkey = ParseConfig::new(test_magic())
            .encode_script_buf(&tag)
            .expect("tag script encodes");

        Transaction {
            version: TxVersion(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey,
            }],
        }
    }

    #[test]
    fn peek_commit_marker_accepts_exact_marker_at_output_zero() {
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let marker = peek_commit_marker(&commit, test_magic())
            .expect("marker parse succeeds")
            .expect("marker");
        assert_eq!(marker.version(), 0);
    }

    proptest! {
        #[test]
        fn peek_commit_marker_rejects_wrong_magic(wrong_magic in magic_bytes_strategy()) {
            prop_assume!(wrong_magic != *b"ALPN");
            let commit = build_commit_tx(MagicBytes::new(wrong_magic), 0, 1, false);
            prop_assert_eq!(peek_commit_marker(&commit, test_magic()).expect("marker parse succeeds"), None);
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
                matches!(err, ScanError::EnvelopeParse { source: DaParseError::MissingReveal(2), .. }),
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
                matches!(err, ScanError::EnvelopeParse { source: DaParseError::DuplicateReveal(1), .. }),
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

        assert!(matches!(err, ScanError::InvalidSequencerPubkey { .. }));
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
            ScanError::UnsupportedCommitMarkerVersion { version: 1, .. }
        ));
    }

    #[test]
    fn parse_chunked_envelope_rejects_malformed_commit_marker() {
        let sequencer_pubkey = test_key(7);
        let mut commit = build_commit_tx(test_magic(), 0, 1, false);
        commit.output[0].script_pubkey = malformed_protocol_marker_script();
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("malformed commit marker must fail");

        assert!(matches!(err, ScanError::MalformedCommitMarker { .. }));
    }

    #[test]
    fn scan_blocks_ignores_unrelated_malformed_op_return() {
        let sequencer_pubkey = test_key(7);
        let blocks = vec![block_data(10, vec![unrelated_malformed_op_return_tx()])];

        let envelopes =
            scan_blocks(&blocks, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert!(envelopes.is_empty());
    }

    #[test]
    fn scan_blocks_ignores_same_magic_non_ee_da_op_return() {
        let sequencer_pubkey = test_key(7);
        let blocks = vec![block_data(10, vec![malformed_same_magic_op_return_tx()])];

        let envelopes =
            scan_blocks(&blocks, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert!(envelopes.is_empty());
    }

    #[test]
    fn scan_blocks_ignores_sps50_checkpoint_like_tx() {
        let sequencer_pubkey = test_key(7);
        let blocks = vec![block_data(10, vec![sps50_checkpoint_like_tx()])];

        let envelopes =
            scan_blocks(&blocks, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert!(envelopes.is_empty());
    }

    #[test]
    fn parse_chunked_envelope_rejects_missing_commit_marker() {
        let sequencer_pubkey = test_key(7);
        let mut commit = build_commit_tx(test_magic(), 0, 1, false);
        commit.output.swap(0, 1);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("missing commit marker must fail");

        assert!(matches!(err, ScanError::MissingCommitMarker { .. }));
    }

    #[test]
    fn parse_chunked_envelope_rejects_missing_reveal_slots() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 0, false);

        let err = parse_chunked_envelope(&commit, &[], test_magic(), sequencer_pubkey)
            .expect_err("missing reveal slots must fail");

        assert!(matches!(
            err,
            ScanError::EnvelopeParse {
                source: DaParseError::MissingRevealSlots,
                ..
            }
        ));
    }

    #[test]
    fn parse_chunked_envelope_rejects_reveal_spending_multiple_slots() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 2, false);
        let mut reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let witness = reveal.input[0].witness.clone();
        reveal.input.push(TxIn {
            previous_output: OutPoint {
                txid: commit.compute_txid(),
                vout: 2,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        });

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("one reveal spending multiple slots must fail");

        assert!(matches!(
            err,
            ScanError::EnvelopeParse {
                source: DaParseError::RevealMultipleCommitSpends,
                ..
            }
        ));
    }

    #[test]
    fn parse_chunked_envelope_rejects_p2tr_change_ambiguity() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, true);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");

        let err = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect_err("ambiguous P2TR change must fail");

        assert!(matches!(
            err,
            ScanError::EnvelopeParse {
                source: DaParseError::AmbiguousTaprootChangeOutput(_),
                ..
            }
        ));
    }

    #[test]
    fn scan_blocks_extracts_commit_and_reveal_from_different_blocks() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let blocks = vec![
            block_data(10, vec![commit.clone()]),
            block_data(11, vec![reveal]),
        ];

        let envelopes =
            scan_blocks(&blocks, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit.compute_txid());
        assert_eq!(envelopes[0].chunks(), vec![b"chunk".to_vec()]);
    }

    #[test]
    fn scan_blocks_rejects_missing_reveal_after_full_range() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let blocks = vec![block_data(10, vec![commit])];

        let err = scan_blocks(&blocks, test_magic(), sequencer_pubkey)
            .expect_err("missing reveal must fail after range finalization");

        assert!(matches!(
            err,
            ScanError::EnvelopeParse {
                source: DaParseError::MissingReveal(1),
                ..
            }
        ));
    }

    #[test]
    fn scan_blocks_rejects_reveal_spending_slots_from_multiple_commits() {
        let sequencer_pubkey = test_key(7);
        let commit0 = build_commit_tx(test_magic(), 0, 1, false);
        let mut commit1 = build_commit_tx(test_magic(), 0, 1, false);
        commit1.input[0].sequence = Sequence::MAX;
        let mut reveal = build_reveal_tx(commit0.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let witness = reveal.input[0].witness.clone();
        reveal.input.push(TxIn {
            previous_output: OutPoint {
                txid: commit1.compute_txid(),
                vout: 1,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        });
        let blocks = vec![
            block_data(10, vec![commit0, commit1]),
            block_data(11, vec![reveal]),
        ];

        let err = scan_blocks(&blocks, test_magic(), sequencer_pubkey)
            .expect_err("multi-commit reveal must fail");

        assert!(matches!(err, ScanError::MultipleRevealCommitSpends { .. }));
    }

    #[test]
    fn scan_blocks_scans_reveal_only_blocks() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let reveal_block = bitcoin::Block {
            header: Header {
                version: Version::from_consensus(1),
                prev_blockhash: bitcoin::BlockHash::all_zeros(),
                merkle_root: TxMerkleNode::all_zeros(),
                time: 0,
                bits: CompactTarget::from_consensus(0),
                nonce: 0,
            },
            txdata: vec![reveal],
        };
        let blocks = vec![
            block_data(10, vec![commit.clone()]),
            L1BlockData::new(
                11,
                bitcoin::BlockHash::from_byte_array([11; 32]),
                reveal_block,
            ),
        ];

        let envelopes =
            scan_blocks(&blocks, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit.compute_txid());
    }

    #[test]
    fn scan_blocks_ignores_commit_non_reveal_output_spends() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let commit_txid = commit.compute_txid();
        let reveal = build_reveal_tx(commit_txid, 1, sequencer_pubkey, b"chunk");
        let change_spend = Transaction {
            version: TxVersion(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: commit_txid,
                    vout: 2,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(500),
                script_pubkey: ScriptBuf::new_p2wpkh(&bitcoin::WPubkeyHash::all_zeros()),
            }],
        };
        let blocks = vec![
            block_data(10, vec![commit.clone()]),
            block_data(11, vec![change_spend, reveal]),
        ];

        let envelopes =
            scan_blocks(&blocks, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit_txid);
        assert_eq!(envelopes[0].chunks(), vec![b"chunk".to_vec()]);
    }

    #[test]
    fn parse_chunked_envelope_ignores_non_slot_input_during_authentication() {
        let sequencer_pubkey = test_key(7);
        let commit = build_commit_tx(test_magic(), 0, 1, false);
        let commit_txid = commit.compute_txid();
        let mut reveal = build_reveal_tx(commit_txid, 1, sequencer_pubkey, b"chunk");
        reveal.input.push(TxIn {
            previous_output: OutPoint {
                txid: commit_txid,
                vout: 2,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        });

        let parsed = parse_chunked_envelope(&commit, &[reveal], test_magic(), sequencer_pubkey)
            .expect("non-slot input does not require DA reveal authentication");

        assert_eq!(parsed.commit_txid(), commit_txid);
        assert_eq!(parsed.chunks(), vec![b"chunk".to_vec()]);
    }

    #[test]
    fn scan_blocks_extracts_btcio_writer_envelope_across_blocks() {
        let keypair = test_keypair(7);
        let sequencer_pubkey = XOnlyPublicKey::from_keypair(&keypair).0;
        let change_address = test_change_address();
        let config = EnvelopeConfig::new(
            test_magic(),
            change_address.clone(),
            Network::Regtest,
            FeeRate::from_sat_per_vb_u32(1_000),
            546,
            None,
        );
        let chunks = vec![b"chunk-0".to_vec(), b"chunk-1".to_vec()];
        let txs = build_chunked_envelope_txs(
            &config,
            &chunks,
            &test_magic(),
            DA_BLOB_VERSION,
            &keypair,
            vec![test_utxo(&change_address)],
        )
        .expect("btcio writer builds envelope");
        let blocks = vec![
            block_data(10, vec![txs.commit_tx.clone()]),
            block_data(11, txs.reveal_txs),
        ];

        let envelopes =
            scan_blocks(&blocks, test_magic(), sequencer_pubkey).expect("scan succeeds");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), txs.commit_tx.compute_txid());
        assert_eq!(envelopes[0].chunks(), chunks);
    }
}
