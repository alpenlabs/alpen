//! Extracts EE DA chunked envelopes from bounded L1 block data.
//!
//! Entry point: [`EnvelopeScanner`]. See its type docs for the
//! pipeline model, accumulator fields, and rejection handling.

use std::collections::BTreeMap;

use alpen_ee_da_types::DA_BLOB_VERSION;
use bitcoin::{secp256k1::XOnlyPublicKey, Block, Transaction, Txid};
use strata_l1_commit_reveal_fmt::{
    extract_payload_for_parsed_commit, scan_commit_reveal_txs,
    CommitRevealParseError as DaParseError, CommitRevealScanRejection, ParsedCommit,
};
use strata_l1_txfmt::MagicBytes;
use thiserror::Error;
use tracing::warn;

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

    /// Iterates over raw encoded DA chunks ordered by commit output vout.
    pub fn iter_chunks(&self) -> impl Iterator<Item = &[u8]> {
        self.chunks.iter().map(Vec::as_slice)
    }

    /// Concatenates all raw encoded DA chunks.
    pub fn to_concat_chunks(&self) -> Vec<u8> {
        self.iter_chunks().flatten().copied().collect()
    }

    /// Consumes the envelope and returns its ordered raw encoded DA chunks.
    pub fn into_chunks(self) -> Vec<Vec<u8>> {
        self.chunks
    }
}

/// Result of scanning one bounded L1 range.
#[derive(Debug, Default)]
pub struct ScanOutcome {
    /// Authenticated, fully parsed envelopes discovered in the range.
    envelopes: Vec<ParsedEnvelope>,
}

impl ScanOutcome {
    fn new(envelopes: Vec<ParsedEnvelope>) -> Self {
        Self { envelopes }
    }

    /// Returns authenticated, fully parsed envelopes.
    pub fn envelopes(&self) -> &[ParsedEnvelope] {
        &self.envelopes
    }
}

/// Reasons a marker-compatible candidate was skipped during range scanning.
#[derive(Debug, Error)]
enum RejectionReason {
    #[error("expected commit marker tail to be 4 bytes, found {found}")]
    UnexpectedMarkerTailLength { found: usize },

    #[error("commit has no reveal slots")]
    MissingRevealSlots,

    #[error("missing reveal for commit output {vout}")]
    MissingReveal { vout: u32 },

    #[error("unauthenticated reveal {reveal_txid}")]
    UnauthenticatedReveal { reveal_txid: Txid },

    #[error("reveal {reveal_txid} spends slots from multiple commits")]
    RevealCrossesCommits { reveal_txid: Txid },

    #[error("unsupported commit marker version {version}")]
    UnsupportedVersion { version: u32 },

    #[error("malformed envelope: {source}")]
    MalformedEnvelope {
        #[source]
        source: DaParseError,
    },
}

/// Errors raised while discovering or parsing EE DA chunked envelopes.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("duplicate commit transaction id {txid}")]
    DuplicateCommitTxid { txid: Txid },
}

#[derive(Debug)]
struct CommitCandidate {
    parsed: ParsedCommit,
}

#[derive(Clone, Debug)]
struct RevealCandidate {
    tx: Transaction,
    txid: Txid,
}

/// Configuration for one EE DA envelope scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvelopeScannerConfig {
    magic_bytes: MagicBytes,
    sequencer_pubkey: XOnlyPublicKey,
}

impl EnvelopeScannerConfig {
    /// Creates scanner configuration for one sequencer key epoch.
    pub fn new(magic_bytes: MagicBytes, sequencer_pubkey: XOnlyPublicKey) -> Self {
        Self {
            magic_bytes,
            sequencer_pubkey,
        }
    }

    /// Returns the L1 marker magic bytes.
    pub fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }

    /// Returns the sequencer public key accepted by reveal leaves.
    pub fn sequencer_pubkey(&self) -> XOnlyPublicKey {
        self.sequencer_pubkey
    }
}

/// Incrementally scans a bounded L1 range for EE DA commit/reveal envelopes.
///
/// [`EnvelopeScannerConfig`] holds the read-only parameters (magic
/// bytes, sequencer pubkey); the accumulator fields collect discovered
/// commit and reveal candidates as [`ingest_block`](Self::ingest_block) walks
/// each L1 block. Full blocks are not retained.
///
/// Per-block pipeline: shared commit/reveal scan → Alpen marker-tail policy →
/// accumulator update → deferred `finalize_commit` at [`finish`](Self::finish),
/// which authenticates each commit's reveals and returns the surviving
/// envelopes.
///
/// Rejected candidates (adversarial or ambiguous) are dropped from
/// `commits_by_txid` on discovery and logged via `tracing::warn!` with
/// `%commit_txid` and `?reason`. Nothing retained; `ScanOutcome`
/// carries only the surviving envelopes.
#[derive(Debug)]
pub struct EnvelopeScanner {
    config: EnvelopeScannerConfig,
    commits_by_txid: BTreeMap<Txid, CommitCandidate>,
    reveals_by_commit: BTreeMap<Txid, BTreeMap<Txid, RevealCandidate>>,
}

impl EnvelopeScanner {
    /// Creates an empty scanner for one confirmed L1 range.
    pub fn new(magic_bytes: MagicBytes, sequencer_pubkey: XOnlyPublicKey) -> Self {
        Self::with_config(EnvelopeScannerConfig::new(magic_bytes, sequencer_pubkey))
    }

    /// Creates an empty scanner from scanner configuration.
    pub fn with_config(config: EnvelopeScannerConfig) -> Self {
        Self {
            config,
            commits_by_txid: BTreeMap::new(),
            reveals_by_commit: BTreeMap::new(),
        }
    }

    /// Ingests one fetched L1 block.
    pub fn ingest_block(&mut self, block: &Block) -> Result<(), ScanError> {
        let existing_commits = self
            .commits_by_txid
            .values()
            .map(|candidate| candidate.parsed.clone())
            .collect::<Vec<_>>();
        let scan = scan_commit_reveal_txs(
            &self.config.magic_bytes,
            existing_commits.iter(),
            block.txdata.iter(),
        );
        let (new_commits, reveal_groups, rejections) = scan.into_parts();

        for rejection in rejections {
            self.reject_scan_rejection(&rejection);
        }

        for parsed in new_commits {
            let txid = parsed.txid();
            match validate_parsed_commit_marker(&parsed) {
                Ok(()) => {
                    let candidate = CommitCandidate { parsed };
                    if self.commits_by_txid.insert(txid, candidate).is_some() {
                        return Err(ScanError::DuplicateCommitTxid { txid });
                    }
                }
                Err(reason) => self.reject_commit(txid, reason),
            }
        }

        for (commit_txid, reveals) in reveal_groups {
            if !self.commits_by_txid.contains_key(&commit_txid) {
                continue;
            }
            let stored_reveals = self.reveals_by_commit.entry(commit_txid).or_default();
            for reveal in reveals {
                let txid = reveal.txid();
                stored_reveals
                    .entry(txid)
                    .or_insert_with(|| RevealCandidate {
                        tx: reveal.tx().clone(),
                        txid,
                    });
            }
        }

        Ok(())
    }

    fn finalize_commit(&self, commit: &CommitCandidate) -> Result<ParsedEnvelope, RejectionReason> {
        let reveals = self
            .reveals_by_commit
            .get(&commit.parsed.txid())
            .map(|reveals| reveals.values().collect::<Vec<_>>())
            .unwrap_or_default();

        authenticate_and_parse_envelope(commit, &reveals, self.config.sequencer_pubkey())
    }

    fn reject_commit(&mut self, commit_txid: Txid, reason: RejectionReason) {
        warn!(%commit_txid, ?reason, "skipping unauthenticated or incomplete EE DA candidate");
        // Remove so it isn't emitted at `finish`. Orphaned entries in
        // `reveals_by_commit` become unreachable but harmless — nothing
        // looks them up after the commit key is gone.
        self.commits_by_txid.remove(&commit_txid);
    }

    /// Finalizes all discovered commits into parsed envelopes.
    ///
    /// Envelopes are returned in commit transaction-id order, not L1 block or
    /// transaction-position order. Marker-compatible candidates that do not
    /// form complete authenticated envelopes are skipped and do not block valid
    /// envelopes in the same range.
    pub fn finish(mut self) -> ScanOutcome {
        let mut envelopes = Vec::new();
        let commit_txids = self.commits_by_txid.keys().copied().collect::<Vec<_>>();

        for commit_txid in commit_txids {
            let outcome = {
                let commit = self
                    .commits_by_txid
                    .get(&commit_txid)
                    .expect("commit txid collected from map key");
                self.finalize_commit(commit)
            };
            match outcome {
                Ok(envelope) => envelopes.push(envelope),
                Err(reason) => self.reject_commit(commit_txid, reason),
            }
        }

        ScanOutcome::new(envelopes)
    }

    fn reject_scan_rejection(&self, rejection: &CommitRevealScanRejection) {
        let txid = rejection.txid();
        match rejection.error() {
            DaParseError::RevealSpansMultipleCommits => {
                let reason = RejectionReason::RevealCrossesCommits { reveal_txid: txid };
                warn!(%txid, ?reason, "skipping malformed EE DA reveal candidate");
            }
            source => {
                warn!(%txid, ?source, "skipping malformed EE DA commit/reveal candidate");
            }
        }
    }
}

/// Authenticates and parses one grouped commit/reveal envelope.
///
/// This delegates commit/reveal parsing to
/// [`extract_payload_for_parsed_commit`], then applies the EE sequencer-pubkey
/// policy.
fn authenticate_and_parse_envelope(
    commit: &CommitCandidate,
    reveals: &[&RevealCandidate],
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<ParsedEnvelope, RejectionReason> {
    let commit_txid = commit.parsed.txid();

    let parsed = extract_payload_for_parsed_commit(&commit.parsed, reveals.iter().map(|r| &r.tx))
        .map_err(convert_parse_error_to_rejection_reason)?;
    let expected_pubkey = sequencer_pubkey.serialize();
    let parsed = parsed
        .authenticate(&expected_pubkey)
        .map_err(|source| convert_auth_error_to_rejection_reason(source, reveals, commit_txid))?;

    Ok(ParsedEnvelope::new(commit_txid, parsed.into_chunks()))
}

fn validate_parsed_commit_marker(commit: &ParsedCommit) -> Result<(), RejectionReason> {
    let Ok(version) = commit.marker_tail_array::<4>() else {
        return Err(RejectionReason::UnexpectedMarkerTailLength {
            found: commit.marker_tail().len(),
        });
    };
    let version = u32::from_be_bytes(*version);
    if version != DA_BLOB_VERSION {
        return Err(RejectionReason::UnsupportedVersion { version });
    }
    Ok(())
}

fn convert_auth_error_to_rejection_reason(
    error: DaParseError,
    reveals: &[&RevealCandidate],
    commit_txid: Txid,
) -> RejectionReason {
    match error {
        DaParseError::UnexpectedRevealPubkey => {
            let reveal_txid = reveals
                .first()
                .map(|reveal| reveal.txid)
                .unwrap_or(commit_txid);
            RejectionReason::UnauthenticatedReveal { reveal_txid }
        }
        source => convert_parse_error_to_rejection_reason(source),
    }
}

fn convert_parse_error_to_rejection_reason(error: DaParseError) -> RejectionReason {
    match error {
        DaParseError::MissingRevealSlots => RejectionReason::MissingRevealSlots,
        DaParseError::MissingReveal { vout } => RejectionReason::MissingReveal { vout },
        source => RejectionReason::MalformedEnvelope { source },
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime,
        block::{Header, Version},
        hashes::Hash,
        opcodes::all::OP_RETURN,
        pow::CompactTarget,
        script::Builder,
        taproot::LeafVersion,
        transaction::Version as TxVersion,
        Amount, BlockHash, OutPoint, ScriptBuf, Sequence, TxIn, TxMerkleNode, TxOut, WPubkeyHash,
        Witness,
    };
    use proptest::prelude::*;
    use strata_l1_commit_reveal_fmt::test_utils as commit_reveal_fixtures;
    use strata_l1_txfmt::{ParseConfig, TagDataRef};

    use super::*;
    use crate::{
        fetch::L1BlockData,
        test_utils::{build_block_with_txs, magic_bytes_strategy, make_deterministic_pubkey},
    };

    const SEQUENCER_KEY_SEED: u8 = 7;
    const NON_SEQUENCER_KEY_SEED: u8 = 8;

    fn scan_preloaded_l1_blocks(
        blocks: &[L1BlockData],
        magic_bytes: MagicBytes,
        sequencer_pubkey: XOnlyPublicKey,
    ) -> Result<ScanOutcome, ScanError> {
        let mut scanner = EnvelopeScanner::new(magic_bytes, sequencer_pubkey);
        for block in blocks {
            scanner.ingest_block(block.block())?;
        }
        Ok(scanner.finish())
    }

    fn make_alpen_magic_bytes() -> MagicBytes {
        "ALPN".parse().expect("valid ASCII magic")
    }

    fn make_sequencer_pubkey() -> XOnlyPublicKey {
        make_deterministic_pubkey(SEQUENCER_KEY_SEED)
    }

    fn make_future_leaf_version() -> LeafVersion {
        LeafVersion::from_consensus(0xc2).expect("valid future leaf version")
    }

    fn make_fetched_l1_block(height: u32, txs: Vec<Transaction>) -> L1BlockData {
        L1BlockData::new(height, build_block_with_txs(txs))
    }

    fn build_commit_marker_with_extra_opcode() -> ScriptBuf {
        let mut payload = [0u8; 8];
        payload[..4].copy_from_slice(make_alpen_magic_bytes().as_bytes());

        Builder::new()
            .push_opcode(OP_RETURN)
            .push_slice(payload)
            .push_opcode(OP_RETURN)
            .into_script()
    }

    fn build_non_da_op_return_tx() -> Transaction {
        commit_reveal_fixtures::build_marker_candidate_tx(
            Builder::new()
                .push_opcode(OP_RETURN)
                .push_slice(*b"NOTE")
                .push_opcode(OP_RETURN)
                .into_script(),
        )
    }

    fn build_magic_prefixed_non_marker_tx() -> Transaction {
        commit_reveal_fixtures::build_marker_candidate_tx(
            Builder::new()
                .push_opcode(OP_RETURN)
                .push_slice(*b"ALPNnot-da")
                .into_script(),
        )
    }

    fn build_sps50_checkpoint_tx() -> Transaction {
        let tag = TagDataRef::new(1, 1, &[]).expect("valid checkpoint-like tag");
        let script_pubkey = ParseConfig::new(make_alpen_magic_bytes())
            .encode_script_buf(&tag)
            .expect("tag script encodes");

        commit_reveal_fixtures::build_marker_candidate_tx(script_pubkey)
    }

    proptest! {
        #[test]
        fn test_wrong_magic_ignored(wrong_magic in magic_bytes_strategy()) {
            prop_assume!(wrong_magic != *b"ALPN");
            let set = commit_reveal_fixtures::build_commit_reveal_set(
                &MagicBytes::new(wrong_magic),
                &DA_BLOB_VERSION.to_be_bytes(),
                &[b"chunk".as_slice()],
                SEQUENCER_KEY_SEED,
            );
            let blocks = vec![make_fetched_l1_block(10, vec![set.commit])];

            let outcome = scan_preloaded_l1_blocks(
                &blocks,
                make_alpen_magic_bytes(),
                make_sequencer_pubkey(),
            )
            .expect("scan succeeds");

            prop_assert!(outcome.envelopes().is_empty());
        }
    }

    #[test]
    fn test_marker_with_extra_opcode_ignored() {
        let tx = commit_reveal_fixtures::build_marker_candidate_tx(
            build_commit_marker_with_extra_opcode(),
        );
        let blocks = vec![make_fetched_l1_block(10, vec![tx])];

        let outcome =
            scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), make_sequencer_pubkey())
                .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_non_da_op_return_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let blocks = vec![make_fetched_l1_block(10, vec![build_non_da_op_return_tx()])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_non_marker_magic_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let blocks = vec![make_fetched_l1_block(
            10,
            vec![build_magic_prefixed_non_marker_tx()],
        )];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_checkpoint_tx_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let blocks = vec![make_fetched_l1_block(10, vec![build_sps50_checkpoint_tx()])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_unsupported_version_candidate_does_not_block_valid_envelope() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let unsupported = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &1u32.to_be_bytes(),
            &[b"unsupported".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let valid = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let blocks = vec![
            make_fetched_l1_block(10, vec![unsupported.commit, valid.commit.clone()]),
            make_fetched_l1_block(11, valid.reveals),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert_eq!(outcome.envelopes().len(), 1);
        assert_eq!(
            outcome.envelopes()[0].commit_txid(),
            valid.commit.compute_txid()
        );
    }

    #[test]
    fn test_marker_with_extra_opcode_does_not_block_valid_envelope() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let unrelated_marker_tx = commit_reveal_fixtures::build_marker_candidate_tx(
            build_commit_marker_with_extra_opcode(),
        );
        let valid = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let blocks = vec![
            make_fetched_l1_block(10, vec![unrelated_marker_tx, valid.commit.clone()]),
            make_fetched_l1_block(11, valid.reveals),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert_eq!(outcome.envelopes().len(), 1);
        assert_eq!(
            outcome.envelopes()[0].commit_txid(),
            valid.commit.compute_txid()
        );
    }

    #[test]
    fn test_wrong_key_candidate_skipped() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            NON_SEQUENCER_KEY_SEED,
        );
        let blocks = vec![
            make_fetched_l1_block(10, vec![set.commit]),
            make_fetched_l1_block(11, set.reveals),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_non_tapscript_leaf_candidate_skipped() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let reveal = commit_reveal_fixtures::build_reveal_tx(vec![
            commit_reveal_fixtures::build_unsupported_leaf_reveal_input(
                set.commit.compute_txid(),
                1,
                b"chunk",
                SEQUENCER_KEY_SEED,
                make_future_leaf_version(),
            ),
        ]);
        let blocks = vec![
            make_fetched_l1_block(10, vec![set.commit]),
            make_fetched_l1_block(11, vec![reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_no_slot_marker_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = commit_reveal_fixtures::build_commit_tx(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            0,
            &[],
        );
        let blocks = vec![make_fetched_l1_block(10, vec![commit])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_missing_marker_ignored() {
        let mut commit = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        )
        .commit;
        commit.output.swap(0, 1);
        let blocks = vec![make_fetched_l1_block(10, vec![commit])];

        let outcome =
            scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), make_sequencer_pubkey())
                .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_cross_block_envelope() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let blocks = vec![
            make_fetched_l1_block(10, vec![set.commit.clone()]),
            make_fetched_l1_block(11, set.reveals),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        let envelopes = outcome.envelopes();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), set.commit.compute_txid());
        assert_eq!(envelopes[0].chunks(), vec![b"chunk".to_vec()]);
    }

    #[test]
    fn test_envelope_txid_order() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set0 = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk-0".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let set1 = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk-1a".as_slice(), b"chunk-1b".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let commit0_txid = set0.commit.compute_txid();
        let commit1_txid = set1.commit.compute_txid();
        let mut reveals = set0.reveals;
        reveals.extend(set1.reveals);
        let blocks = vec![
            make_fetched_l1_block(10, vec![set0.commit, set1.commit]),
            make_fetched_l1_block(11, reveals),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");
        let envelopes = outcome.envelopes();
        let commit_txids = envelopes
            .iter()
            .map(ParsedEnvelope::commit_txid)
            .collect::<Vec<_>>();

        assert_eq!(envelopes.len(), 2);
        assert_eq!(commit_txids[0], commit0_txid.min(commit1_txid));
        assert_eq!(commit_txids[1], commit0_txid.max(commit1_txid));
    }

    #[test]
    fn test_missing_reveal_candidate_skipped() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let blocks = vec![make_fetched_l1_block(10, vec![set.commit])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_cross_commit_reveal_candidate_skipped() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set0 = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk-0".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let set1 = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk-1a".as_slice(), b"chunk-1b".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let mut reveal = set0.reveals[0].clone();
        let witness = reveal.input[0].witness.clone();
        reveal.input.push(TxIn {
            previous_output: OutPoint {
                txid: set1.commit.compute_txid(),
                vout: 1,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        });
        let blocks = vec![
            make_fetched_l1_block(10, vec![set0.commit, set1.commit]),
            make_fetched_l1_block(11, vec![reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
    }

    #[test]
    fn test_malformed_envelope_isolated() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let malformed = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk-0".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let valid = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"valid-a".as_slice(), b"valid-b".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let malformed_reveal0 = malformed.reveals[0].clone();
        let mut malformed_reveal1 = commit_reveal_fixtures::build_reveal_tx(vec![
            commit_reveal_fixtures::build_reveal_input(
                malformed.commit.compute_txid(),
                1,
                Some(b"chunk-1"),
                SEQUENCER_KEY_SEED,
            ),
        ]);
        malformed_reveal1.input[0].sequence = Sequence::MAX;
        let mut reveals = vec![malformed_reveal0, malformed_reveal1];
        reveals.extend(valid.reveals);
        let blocks = vec![
            make_fetched_l1_block(10, vec![malformed.commit, valid.commit.clone()]),
            make_fetched_l1_block(11, reveals),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert_eq!(outcome.envelopes().len(), 1);
        assert_eq!(
            outcome.envelopes()[0].commit_txid(),
            valid.commit.compute_txid()
        );
    }

    #[test]
    fn test_duplicate_commit_txid_rejected() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let blocks = vec![
            make_fetched_l1_block(10, vec![set.commit.clone()]),
            make_fetched_l1_block(11, vec![set.commit.clone()]),
        ];

        let err = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect_err("duplicate commit txid must fail");

        assert!(matches!(
            err,
            ScanError::DuplicateCommitTxid { txid } if txid == set.commit.compute_txid()
        ));
    }

    #[test]
    fn test_non_commit_blocks_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let reveal_block = Block {
            header: Header {
                version: Version::from_consensus(1),
                prev_blockhash: BlockHash::all_zeros(),
                merkle_root: TxMerkleNode::all_zeros(),
                time: 0,
                bits: CompactTarget::from_consensus(0),
                nonce: 0,
            },
            txdata: set.reveals,
        };
        let blocks = vec![
            make_fetched_l1_block(10, vec![set.commit.clone()]),
            L1BlockData::new(11, reveal_block),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        let envelopes = outcome.envelopes();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), set.commit.compute_txid());
    }

    #[test]
    fn test_change_spend_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let set = commit_reveal_fixtures::build_commit_reveal_set(
            &make_alpen_magic_bytes(),
            &DA_BLOB_VERSION.to_be_bytes(),
            &[b"chunk".as_slice()],
            SEQUENCER_KEY_SEED,
        );
        let commit_txid = set.commit.compute_txid();
        let reveal = set.reveals[0].clone();
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
                script_pubkey: ScriptBuf::new_p2wpkh(&WPubkeyHash::all_zeros()),
            }],
        };
        let blocks = vec![
            make_fetched_l1_block(10, vec![set.commit]),
            make_fetched_l1_block(11, vec![change_spend, reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        let envelopes = outcome.envelopes();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit_txid);
        assert_eq!(envelopes[0].chunks(), vec![b"chunk".to_vec()]);
    }
}
