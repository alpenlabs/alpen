//! Extracts EE DA chunked envelopes from bounded L1 block data.

use std::{
    collections::{BTreeMap, BTreeSet},
    iter::once,
};

use alpen_ee_da_types::{
    extract_da_chunks, last_commit_reveal_vout, DaParseError, DA_BLOB_VERSION,
};
use bitcoin::{
    opcodes::{
        all::{OP_CHECKSIG, OP_ENDIF, OP_IF, OP_RETURN},
        OP_FALSE,
    },
    script::Instruction,
    secp256k1::XOnlyPublicKey,
    taproot::LeafVersion,
    Block, Script, Transaction, Txid,
};
use strata_l1_txfmt::MagicBytes;
use thiserror::Error;

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

/// Inclusive commit-output range containing reveal slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RevealSlotRange {
    last_vout: u32,
}

impl RevealSlotRange {
    fn new(last_vout: u32) -> Option<Self> {
        (last_vout > 0).then_some(Self { last_vout })
    }

    fn last_vout(self) -> u32 {
        self.last_vout
    }

    fn contains_vout(self, vout: u32) -> bool {
        (1..=self.last_vout).contains(&vout)
    }
}

/// Result of matching only the vout-0 commit-marker script.
#[derive(Debug)]
enum CommitMarkerMatch {
    Unrelated,
    MalformedSameMagic { source: DaParseError },
    Supported(CommitMarker),
    Unsupported { version: u32 },
}

/// Transaction classification used before scanner candidate handling.
#[derive(Debug)]
enum TxClassification {
    Unrelated,
    Commit {
        marker: CommitMarker,
        reveal_slots: RevealSlotRange,
    },
    QuarantinedCommit {
        reason: QuarantineReason,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum RevealSpendMatch {
    None,
    One { commit_txid: Txid },
    Multiple { commit_txids: Vec<Txid> },
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

/// Result of scanning one bounded L1 range.
#[derive(Debug, Default)]
pub struct ScanOutcome {
    /// Authenticated, fully parsed envelopes discovered in the range.
    envelopes: Vec<ParsedEnvelope>,

    /// Marker-compatible candidates skipped during envelope parsing.
    quarantined: Vec<QuarantinedCandidate>,
}

impl ScanOutcome {
    fn new(envelopes: Vec<ParsedEnvelope>, quarantined: Vec<QuarantinedCandidate>) -> Self {
        Self {
            envelopes,
            quarantined,
        }
    }

    /// Returns authenticated, fully parsed envelopes.
    pub fn envelopes(&self) -> &[ParsedEnvelope] {
        &self.envelopes
    }

    /// Returns marker-compatible candidates that did not form valid envelopes.
    pub fn quarantined(&self) -> &[QuarantinedCandidate] {
        &self.quarantined
    }

    /// Consumes the outcome and returns its envelopes and quarantined candidates.
    pub fn into_parts(self) -> (Vec<ParsedEnvelope>, Vec<QuarantinedCandidate>) {
        (self.envelopes, self.quarantined)
    }
}

/// Marker-compatible candidate skipped during range scanning.
#[derive(Debug)]
pub struct QuarantinedCandidate {
    commit_txid: Txid,
    reason: QuarantineReason,
}

impl QuarantinedCandidate {
    fn new(commit_txid: Txid, reason: QuarantineReason) -> Self {
        Self {
            commit_txid,
            reason,
        }
    }

    /// Returns the candidate commit transaction id.
    pub fn commit_txid(&self) -> Txid {
        self.commit_txid
    }

    /// Returns why the candidate was skipped.
    pub fn reason(&self) -> &QuarantineReason {
        &self.reason
    }
}

/// Reasons a marker-compatible candidate was skipped during range scanning.
#[derive(Debug, Error)]
pub enum QuarantineReason {
    #[error("commit has no reveal slots")]
    MissingRevealSlots,

    #[error("missing reveals: expected {expected_slots}, covered {covered_slots}")]
    MissingReveals {
        expected_slots: u32,
        covered_slots: u32,
    },

    #[error("unauthenticated reveal {reveal_txid}")]
    UnauthenticatedReveal { reveal_txid: Txid },

    #[error("reveal {reveal_txid} spends slots from multiple commits")]
    RevealCrossesCommits { reveal_txid: Txid },

    #[error("malformed commit marker: {source}")]
    MalformedMarker {
        #[source]
        source: DaParseError,
    },

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
    tx: Transaction,
    txid: Txid,
    reveal_slots: RevealSlotRange,
}

#[derive(Clone, Debug)]
struct RevealCandidate {
    tx: Transaction,
    txid: Txid,
}

#[derive(Debug)]
enum RevealAuthError {
    MissingLeafScript { reveal_txid: Txid },
    UnsupportedLeafVersion { reveal_txid: Txid },
    InvalidSequencerPubkey { reveal_txid: Txid },
}

/// Incrementally scans a bounded L1 range for EE DA commit/reveal envelopes.
///
/// The scanner keeps only discovered commit and reveal candidate transactions;
/// it does not retain every fetched Bitcoin block.
#[derive(Debug)]
pub struct EeDaEnvelopeScanner {
    magic_bytes: MagicBytes,
    sequencer_pubkey: XOnlyPublicKey,
    commits_by_txid: BTreeMap<Txid, CommitCandidate>,
    reveals_by_commit: BTreeMap<Txid, BTreeMap<Txid, RevealCandidate>>,
    quarantined: Vec<QuarantinedCandidate>,
}

impl EeDaEnvelopeScanner {
    /// Creates an empty scanner for one confirmed L1 range.
    pub fn new(magic_bytes: MagicBytes, sequencer_pubkey: XOnlyPublicKey) -> Self {
        Self {
            magic_bytes,
            sequencer_pubkey,
            commits_by_txid: BTreeMap::new(),
            reveals_by_commit: BTreeMap::new(),
            quarantined: Vec::new(),
        }
    }

    /// Ingests one fetched L1 block.
    pub fn ingest_block(&mut self, block: &Block) -> Result<(), ScanError> {
        for tx in &block.txdata {
            self.ingest_transaction(tx)?;
        }
        Ok(())
    }

    fn ingest_transaction(&mut self, tx: &Transaction) -> Result<(), ScanError> {
        let txid = tx.compute_txid();

        match classify_transaction(tx, self.magic_bytes) {
            TxClassification::Unrelated => {}
            TxClassification::Commit {
                marker,
                reveal_slots,
            } => {
                debug_assert_eq!(marker.version(), DA_BLOB_VERSION);
                let candidate = CommitCandidate {
                    tx: tx.clone(),
                    txid,
                    reveal_slots,
                };
                if self.commits_by_txid.insert(txid, candidate).is_some() {
                    return Err(ScanError::DuplicateCommitTxid { txid });
                }
            }
            TxClassification::QuarantinedCommit { reason } => {
                self.quarantined
                    .push(QuarantinedCandidate::new(txid, reason));
            }
        }

        // Bitcoin blocks order parents before children, so same-block reveals
        // cannot appear before the commit output they spend. Check every
        // transaction here: a tx can both publish a new commit marker and spend
        // an earlier commit's reveal slot.
        match match_reveal_spends(tx, &self.commits_by_txid) {
            RevealSpendMatch::None => {}
            RevealSpendMatch::One { commit_txid } => {
                self.reveals_by_commit
                    .entry(commit_txid)
                    .or_default()
                    .entry(txid)
                    .or_insert_with(|| RevealCandidate {
                        tx: tx.clone(),
                        txid,
                    });
            }
            RevealSpendMatch::Multiple { commit_txids } => {
                for commit_txid in commit_txids {
                    self.quarantined.push(QuarantinedCandidate::new(
                        commit_txid,
                        QuarantineReason::RevealCrossesCommits { reveal_txid: txid },
                    ));
                }
            }
        }

        Ok(())
    }

    fn finalize_commit(
        &self,
        commit: &CommitCandidate,
    ) -> Result<ParsedEnvelope, QuarantineReason> {
        let reveals = self
            .reveals_by_commit
            .get(&commit.txid)
            .map(|reveals| reveals.values().collect::<Vec<_>>())
            .unwrap_or_default();

        authenticate_and_parse_envelope(commit, &reveals, self.sequencer_pubkey)
    }

    /// Finalizes all discovered commits into parsed envelopes.
    ///
    /// Envelopes are returned in commit transaction-id order, not L1 block or
    /// transaction-position order. Marker-compatible candidates that do not
    /// form complete authenticated envelopes are returned as quarantined
    /// entries and do not block valid envelopes in the same range.
    pub fn finish(mut self) -> ScanOutcome {
        let mut envelopes = Vec::new();
        let commit_txids = self.commits_by_txid.keys().copied().collect::<Vec<_>>();
        let quarantined_commit_txids = self
            .quarantined
            .iter()
            .map(QuarantinedCandidate::commit_txid)
            .collect::<BTreeSet<_>>();

        for commit_txid in commit_txids {
            if quarantined_commit_txids.contains(&commit_txid) {
                continue;
            }
            let commit = self
                .commits_by_txid
                .get(&commit_txid)
                .expect("commit txid collected from map key");
            match self.finalize_commit(commit) {
                Ok(envelope) => envelopes.push(envelope),
                Err(reason) => self
                    .quarantined
                    .push(QuarantinedCandidate::new(commit_txid, reason)),
            }
        }

        ScanOutcome::new(envelopes, self.quarantined)
    }
}

fn parse_commit_marker_payload(payload: [u8; 8]) -> CommitMarkerMatch {
    let mut version = [0u8; 4];
    version.copy_from_slice(&payload[4..]);
    let version = u32::from_be_bytes(version);
    if version != DA_BLOB_VERSION {
        return CommitMarkerMatch::Unsupported { version };
    }

    CommitMarkerMatch::Supported(CommitMarker { version })
}

/// Classifies a transaction against the EE DA commit marker shape.
///
/// The stable marker discriminator is `magic || version`: bytes `0..4` are
/// [`MagicBytes`], and bytes `4..8` are the big-endian version. The deployed v0
/// marker is exactly 8 bytes. Same-magic OP_RETURN payloads with any other
/// length are unrelated Alpen traffic, while exact marker payloads with extra
/// opcodes are malformed DA markers.
fn classify_transaction(tx: &Transaction, magic_bytes: MagicBytes) -> TxClassification {
    let marker = match match_commit_marker(tx, magic_bytes) {
        CommitMarkerMatch::Unrelated => return TxClassification::Unrelated,
        CommitMarkerMatch::MalformedSameMagic { source } => {
            return TxClassification::QuarantinedCommit {
                reason: QuarantineReason::MalformedMarker { source },
            };
        }
        CommitMarkerMatch::Unsupported { version } => {
            return TxClassification::QuarantinedCommit {
                reason: QuarantineReason::UnsupportedVersion { version },
            };
        }
        CommitMarkerMatch::Supported(marker) => marker,
    };

    let reveal_slots = match last_commit_reveal_vout(tx) {
        Ok(Some(last_reveal_vout)) => RevealSlotRange::new(last_reveal_vout),
        Ok(None) => None,
        Err(source) => {
            return TxClassification::QuarantinedCommit {
                reason: QuarantineReason::MalformedEnvelope { source },
            };
        }
    };
    let Some(reveal_slots) = reveal_slots else {
        return TxClassification::QuarantinedCommit {
            reason: QuarantineReason::MissingRevealSlots,
        };
    };

    TxClassification::Commit {
        marker,
        reveal_slots,
    }
}

fn match_commit_marker(tx: &Transaction, magic_bytes: MagicBytes) -> CommitMarkerMatch {
    let Some(first_output) = tx.output.first() else {
        return CommitMarkerMatch::Unrelated;
    };
    let mut instructions = first_output.script_pubkey.instructions();
    let Some(push) = (match (instructions.next(), instructions.next()) {
        (Some(Ok(Instruction::Op(OP_RETURN))), Some(Ok(Instruction::PushBytes(push))))
            if push.as_bytes().starts_with(magic_bytes.as_bytes())
                && push.as_bytes().len() == 8 =>
        {
            Some(push)
        }
        _ => None,
    }) else {
        return CommitMarkerMatch::Unrelated;
    };
    if instructions.next().is_some() {
        return CommitMarkerMatch::MalformedSameMagic {
            source: DaParseError::MalformedCommitMarker,
        };
    }

    let mut payload = [0u8; 8];
    payload.copy_from_slice(push.as_bytes());
    parse_commit_marker_payload(payload)
}

fn match_reveal_spends(
    tx: &Transaction,
    commits_by_txid: &BTreeMap<Txid, CommitCandidate>,
) -> RevealSpendMatch {
    let mut commit_txids = Vec::new();
    for input in &tx.input {
        let commit_txid = input.previous_output.txid;
        let Some(commit) = commits_by_txid.get(&commit_txid) else {
            continue;
        };
        if !commit
            .reveal_slots
            .contains_vout(input.previous_output.vout)
        {
            continue;
        }

        if !commit_txids.contains(&commit_txid) {
            commit_txids.push(commit_txid);
        }
    }

    match commit_txids.as_slice() {
        [] => RevealSpendMatch::None,
        [commit_txid] => RevealSpendMatch::One {
            commit_txid: *commit_txid,
        },
        _ => RevealSpendMatch::Multiple { commit_txids },
    }
}

/// Authenticates and parses one grouped commit/reveal envelope.
///
/// This checks reveal completeness, validates every reveal script against the
/// configured sequencer key, delegates chunk extraction to
/// [`extract_da_chunks`], and maps per-envelope parse failures to
/// [`QuarantineReason`].
fn authenticate_and_parse_envelope(
    commit: &CommitCandidate,
    reveals: &[&RevealCandidate],
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<ParsedEnvelope, QuarantineReason> {
    let commit_txid = commit.txid;
    let reveal_slots = commit.reveal_slots;

    let covered_slots = count_covered_reveal_slots(commit_txid, reveal_slots, reveals);
    if covered_slots < reveal_slots.last_vout() {
        return Err(QuarantineReason::MissingReveals {
            expected_slots: reveal_slots.last_vout(),
            covered_slots,
        });
    }

    for reveal in reveals {
        authenticate_reveal(
            &reveal.tx,
            reveal.txid,
            commit_txid,
            reveal_slots,
            sequencer_pubkey,
        )
        .map_err(quarantine_reason_from_auth_error)?;
    }

    let chunks = extract_da_chunks(once(&commit.tx).chain(reveals.iter().map(|r| &r.tx)))
        .map_err(|source| QuarantineReason::MalformedEnvelope { source })?;

    Ok(ParsedEnvelope::new(commit_txid, chunks))
}

fn count_covered_reveal_slots(
    commit_txid: Txid,
    reveal_slots: RevealSlotRange,
    reveals: &[&RevealCandidate],
) -> u32 {
    let mut covered_slots = Vec::new();
    for reveal in reveals {
        for input in &reveal.tx.input {
            if input.previous_output.txid != commit_txid {
                continue;
            }
            if !reveal_slots.contains_vout(input.previous_output.vout) {
                continue;
            }
            if !covered_slots.contains(&input.previous_output.vout) {
                covered_slots.push(input.previous_output.vout);
            }
        }
    }

    covered_slots.len() as u32
}

fn quarantine_reason_from_auth_error(error: RevealAuthError) -> QuarantineReason {
    match error {
        RevealAuthError::MissingLeafScript { reveal_txid }
        | RevealAuthError::UnsupportedLeafVersion { reveal_txid }
        | RevealAuthError::InvalidSequencerPubkey { reveal_txid } => {
            QuarantineReason::UnauthenticatedReveal { reveal_txid }
        }
    }
}

fn authenticate_reveal(
    reveal: &Transaction,
    reveal_txid: Txid,
    commit_txid: Txid,
    reveal_slots: RevealSlotRange,
    sequencer_pubkey: XOnlyPublicKey,
) -> Result<(), RevealAuthError> {
    // Callers group reveal candidates by in-range commit-slot spends before
    // authenticating. Keep this check visible so future direct callers don't
    // accidentally treat "no matching input" as a successful auth result.
    let mut matching_input_count = 0usize;

    for input in &reveal.input {
        if input.previous_output.txid != commit_txid {
            continue;
        }
        if !reveal_slots.contains_vout(input.previous_output.vout) {
            continue;
        }
        matching_input_count += 1;

        let leaf = input
            .witness
            .taproot_leaf_script()
            .ok_or(RevealAuthError::MissingLeafScript { reveal_txid })?;
        if leaf.version != LeafVersion::TapScript {
            return Err(RevealAuthError::UnsupportedLeafVersion { reveal_txid });
        }
        let leaf_script = leaf.script.to_owned();

        if !script_matches_sequencer_envelope(&leaf_script, sequencer_pubkey) {
            return Err(RevealAuthError::InvalidSequencerPubkey { reveal_txid });
        }
    }

    debug_assert!(
        matching_input_count > 0,
        "authenticate_reveal called for a reveal not grouped by commit slot"
    );

    Ok(())
}

fn script_matches_sequencer_envelope(script: &Script, sequencer_pubkey: XOnlyPublicKey) -> bool {
    // Envelope body elements are raw byte pushes. Minimal-instruction parsing
    // would reject valid one-byte payload chunks such as `[1]`.
    let mut instructions = script.instructions();

    match instructions.next() {
        Some(Ok(Instruction::PushBytes(bytes)))
            if bytes.as_bytes() == sequencer_pubkey.serialize() => {}
        _ => return false,
    }

    if !matches!(instructions.next(), Some(Ok(Instruction::Op(OP_CHECKSIG)))) {
        return false;
    }
    match instructions.next() {
        Some(Ok(Instruction::Op(op))) if op == OP_FALSE => {}
        Some(Ok(Instruction::PushBytes(bytes))) if bytes.as_bytes().is_empty() => {}
        _ => return false,
    }
    if !matches!(instructions.next(), Some(Ok(Instruction::Op(OP_IF)))) {
        return false;
    }

    loop {
        match instructions.next() {
            Some(Ok(Instruction::PushBytes(_))) => {}
            Some(Ok(Instruction::Op(OP_ENDIF))) => break,
            _ => return false,
        }
    }

    instructions.next().is_none()
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime,
        block::{Header, Version},
        hashes::Hash,
        opcodes::{
            all::{OP_CHECKSIG, OP_DROP, OP_ENDIF, OP_IF, OP_NOT},
            OP_FALSE, OP_TRUE,
        },
        pow::CompactTarget,
        script::Builder,
        secp256k1::Keypair,
        taproot::LeafVersion,
        transaction::Version as TxVersion,
        Address, Amount, BlockHash, FeeRate, Network, OutPoint, ScriptBuf, Sequence, TxIn,
        TxMerkleNode, TxOut, Txid, WPubkeyHash, Witness,
    };
    use bitcoind_async_client::corepc_types::model::ListUnspentItem;
    use proptest::prelude::*;
    use strata_btcio::writer::{
        builder::EnvelopeConfig, chunked_envelope::build_chunked_envelope_txs,
    };
    use strata_l1_txfmt::{ParseConfig, TagDataRef};

    use super::*;
    use crate::{
        fetch::L1BlockData,
        test_utils::{
            build_block_with_txs, build_commit_tx, build_reveal_tx, chunk_body_strategy,
            magic_bytes_strategy, make_deterministic_keypair, make_deterministic_pubkey,
        },
    };

    fn scan_preloaded_l1_blocks(
        blocks: &[L1BlockData],
        magic_bytes: MagicBytes,
        sequencer_pubkey: XOnlyPublicKey,
    ) -> Result<ScanOutcome, ScanError> {
        let mut scanner = EeDaEnvelopeScanner::new(magic_bytes, sequencer_pubkey);
        for block in blocks {
            scanner.ingest_block(block.block())?;
        }
        Ok(scanner.finish())
    }

    fn make_alpen_magic_bytes() -> MagicBytes {
        "ALPN".parse().expect("valid ASCII magic")
    }

    fn make_sequencer_pubkey() -> XOnlyPublicKey {
        make_deterministic_pubkey(7)
    }

    fn make_non_sequencer_pubkey() -> XOnlyPublicKey {
        make_deterministic_pubkey(8)
    }

    fn make_sequencer_keypair() -> Keypair {
        make_deterministic_keypair(7)
    }

    fn make_commit_candidate(tx: Transaction) -> CommitCandidate {
        let txid = tx.compute_txid();
        let TxClassification::Commit { reveal_slots, .. } =
            classify_transaction(&tx, make_alpen_magic_bytes())
        else {
            panic!("test commit must classify as a valid commit");
        };

        CommitCandidate {
            tx,
            txid,
            reveal_slots,
        }
    }

    fn make_reveal_candidate(tx: Transaction) -> RevealCandidate {
        RevealCandidate {
            txid: tx.compute_txid(),
            tx,
        }
    }

    fn make_reveal_candidates(txs: Vec<Transaction>) -> Vec<RevealCandidate> {
        txs.into_iter().map(make_reveal_candidate).collect()
    }

    fn build_reveal_refs(reveals: &[RevealCandidate]) -> Vec<&RevealCandidate> {
        reveals.iter().collect()
    }

    fn build_reveal_with_leaf_script(
        mut reveal: Transaction,
        leaf_script: ScriptBuf,
    ) -> Transaction {
        let control_block = reveal
            .input
            .first()
            .and_then(|input| input.witness.iter().last())
            .expect("standard reveal has control block")
            .to_vec();

        reveal.input[0].witness = Witness::new();
        reveal.input[0].witness.push(Vec::<u8>::new());
        reveal.input[0].witness.push(leaf_script);
        reveal.input[0].witness.push(control_block);
        reveal
    }

    fn build_reveal_with_leaf_version(
        mut reveal: Transaction,
        leaf_version: LeafVersion,
    ) -> Transaction {
        let mut witness_elements = reveal.input[0]
            .witness
            .iter()
            .map(|element| element.to_vec())
            .collect::<Vec<_>>();
        let control_block = witness_elements
            .last_mut()
            .expect("standard reveal has control block");
        control_block[0] = (control_block[0] & 1) | leaf_version.to_consensus();

        let mut witness = Witness::new();
        for element in witness_elements {
            witness.push(element);
        }
        reveal.input[0].witness = witness;
        reveal
    }

    fn make_future_leaf_version() -> LeafVersion {
        LeafVersion::from_consensus(0xc2).expect("valid future leaf version")
    }

    fn make_distinct_commit_tx() -> Transaction {
        let mut commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        commit.input[0].sequence = Sequence::MAX;
        commit
    }

    fn make_writer_change_address() -> Address {
        let script = ScriptBuf::new_p2wpkh(&WPubkeyHash::all_zeros());
        Address::from_script(&script, Network::Regtest).expect("valid P2WPKH address")
    }

    fn make_writer_funding_utxo(address: &Address) -> ListUnspentItem {
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

    fn make_fetched_l1_block(height: u32, txs: Vec<Transaction>) -> L1BlockData {
        L1BlockData::new(
            height,
            BlockHash::from_byte_array([height as u8; 32]),
            build_block_with_txs(txs),
        )
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

    fn build_magic_prefixed_non_marker_tx() -> Transaction {
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

    fn build_sps50_checkpoint_tx() -> Transaction {
        let tag = TagDataRef::new(1, 1, &[]).expect("valid checkpoint-like tag");
        let script_pubkey = ParseConfig::new(make_alpen_magic_bytes())
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
    fn test_valid_marker() {
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        assert!(matches!(
            classify_transaction(&commit, make_alpen_magic_bytes()),
            TxClassification::Commit { marker, .. } if marker.version() == 0
        ));
    }

    proptest! {
        #[test]
        fn test_wrong_magic_ignored(wrong_magic in magic_bytes_strategy()) {
            prop_assume!(wrong_magic != *b"ALPN");
            let commit = build_commit_tx(MagicBytes::new(wrong_magic), 0, 1, false);
            prop_assert!(matches!(
                classify_transaction(&commit, make_alpen_magic_bytes()),
                TxClassification::Unrelated
            ));
        }

        #[test]
        fn test_chunk_order(
            chunk0 in chunk_body_strategy(32),
            chunk1 in chunk_body_strategy(32),
        ) {
            let sequencer_pubkey = make_sequencer_pubkey();
            let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 2, false);
            let commit_txid = commit.compute_txid();
            let reveal1 = build_reveal_tx(commit_txid, 2, sequencer_pubkey, &chunk1);
            let reveal0 = build_reveal_tx(commit_txid, 1, sequencer_pubkey, &chunk0);
            let commit = make_commit_candidate(commit);
            let reveals = make_reveal_candidates(vec![reveal1, reveal0]);
            let reveal_refs = build_reveal_refs(&reveals);
            let parsed = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
                .expect("envelope parses");

            prop_assert_eq!(parsed.commit_txid(), commit_txid);
            prop_assert_eq!(parsed.chunks(), vec![chunk0, chunk1]);
        }

        #[test]
        fn test_missing_reveal_rejected(chunk in chunk_body_strategy(32)) {
            let sequencer_pubkey = make_sequencer_pubkey();
            let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 2, false);
            let reveal0 = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, &chunk);
            let commit = make_commit_candidate(commit);
            let reveals = make_reveal_candidates(vec![reveal0]);
            let reveal_refs = build_reveal_refs(&reveals);
            let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
                .expect_err("missing reveal must fail");

            prop_assert!(
                matches!(err, QuarantineReason::MissingReveals { expected_slots: 2, covered_slots: 1 }),
                "expected missing reveal error, got {err:?}"
            );
        }

        #[test]
        fn test_duplicate_reveal_rejected(
            chunk0 in chunk_body_strategy(32),
            chunk1 in chunk_body_strategy(32),
        ) {
            let sequencer_pubkey = make_sequencer_pubkey();
            let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
            let commit_txid = commit.compute_txid();
            let reveal0 = build_reveal_tx(commit_txid, 1, sequencer_pubkey, &chunk0);
            let reveal1 = build_reveal_tx(commit_txid, 1, sequencer_pubkey, &chunk1);
            let commit = make_commit_candidate(commit);
            let reveals = make_reveal_candidates(vec![reveal0, reveal1]);
            let reveal_refs = build_reveal_refs(&reveals);
            let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
                .expect_err("duplicate reveal must fail");

            prop_assert!(
                matches!(err, QuarantineReason::MalformedEnvelope { source: DaParseError::DuplicateReveal { vout: 1 } }),
                "expected duplicate reveal error, got {err:?}"
            );
        }
    }

    #[test]
    fn test_wrong_key_rejected() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let non_sequencer_pubkey = make_non_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, non_sequencer_pubkey, b"chunk");
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect_err("wrong sequencer pubkey must fail");

        assert!(matches!(
            err,
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_single_byte_chunk() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, &[1]);
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let parsed = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect("single-byte payload chunk must parse");

        assert_eq!(parsed.chunks(), &[vec![1]]);
    }

    #[test]
    fn test_checksig_override() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let malicious_script = Builder::new()
            .push_slice(sequencer_pubkey.serialize())
            .push_opcode(OP_CHECKSIG)
            .push_opcode(OP_DROP)
            .push_opcode(OP_TRUE)
            .push_opcode(OP_FALSE)
            .push_opcode(OP_IF)
            .push_slice(*b"chunk")
            .push_opcode(OP_ENDIF)
            .into_script();
        let reveal = build_reveal_with_leaf_script(
            build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk"),
            malicious_script,
        );
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect_err("checksig result override must fail authentication");

        assert!(matches!(
            err,
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_checksig_inversion() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let malicious_script = Builder::new()
            .push_slice(sequencer_pubkey.serialize())
            .push_opcode(OP_CHECKSIG)
            .push_opcode(OP_NOT)
            .push_opcode(OP_IF)
            .push_slice(*b"chunk")
            .push_opcode(OP_ENDIF)
            .into_script();
        let reveal = build_reveal_with_leaf_script(
            build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk"),
            malicious_script,
        );
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect_err("checksig result inversion must fail authentication");

        assert!(matches!(
            err,
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_trailing_opcode() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let malicious_script = Builder::new()
            .push_slice(sequencer_pubkey.serialize())
            .push_opcode(OP_CHECKSIG)
            .push_opcode(OP_FALSE)
            .push_opcode(OP_IF)
            .push_slice(*b"chunk")
            .push_opcode(OP_ENDIF)
            .push_opcode(OP_DROP)
            .into_script();
        let reveal = build_reveal_with_leaf_script(
            build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk"),
            malicious_script,
        );
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect_err("trailing opcode after envelope must fail authentication");

        assert!(matches!(
            err,
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_non_push_body_opcode() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let malicious_script = Builder::new()
            .push_slice(sequencer_pubkey.serialize())
            .push_opcode(OP_CHECKSIG)
            .push_opcode(OP_FALSE)
            .push_opcode(OP_IF)
            .push_opcode(OP_TRUE)
            .push_slice(*b"chunk")
            .push_opcode(OP_ENDIF)
            .into_script();
        let reveal = build_reveal_with_leaf_script(
            build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk"),
            malicious_script,
        );
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect_err("non-push opcode inside envelope must fail authentication");

        assert!(matches!(
            err,
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_missing_false_before_if() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let malicious_script = Builder::new()
            .push_slice(sequencer_pubkey.serialize())
            .push_opcode(OP_CHECKSIG)
            .push_opcode(OP_IF)
            .push_slice(*b"chunk")
            .push_opcode(OP_ENDIF)
            .into_script();
        let reveal = build_reveal_with_leaf_script(
            build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk"),
            malicious_script,
        );
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect_err("missing OP_FALSE before OP_IF must fail authentication");

        assert!(matches!(
            err,
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_unsupported_version_quarantined() {
        let commit = build_commit_tx(make_alpen_magic_bytes(), 1, 1, false);

        let TxClassification::QuarantinedCommit { reason } =
            classify_transaction(&commit, make_alpen_magic_bytes())
        else {
            panic!("unsupported version must quarantine the commit");
        };
        assert!(matches!(
            reason,
            QuarantineReason::UnsupportedVersion { version: 1 }
        ));
    }

    #[test]
    fn test_malformed_marker_quarantined() {
        let mut commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        commit.output[0].script_pubkey = build_commit_marker_with_extra_opcode();

        let TxClassification::QuarantinedCommit { reason } =
            classify_transaction(&commit, make_alpen_magic_bytes())
        else {
            panic!("malformed marker must quarantine the commit");
        };
        assert!(matches!(reason, QuarantineReason::MalformedMarker { .. }));
    }

    #[test]
    fn test_non_da_op_return_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let blocks = vec![make_fetched_l1_block(10, vec![build_non_da_op_return_tx()])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
        assert!(outcome.quarantined().is_empty());
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
        assert!(outcome.quarantined().is_empty());
    }

    #[test]
    fn test_checkpoint_tx_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let blocks = vec![make_fetched_l1_block(10, vec![build_sps50_checkpoint_tx()])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
        assert!(outcome.quarantined().is_empty());
    }

    #[test]
    fn test_unsupported_version_isolated() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let unsupported_commit = build_commit_tx(make_alpen_magic_bytes(), 1, 1, false);
        let valid_commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let valid_reveal =
            build_reveal_tx(valid_commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let blocks = vec![
            make_fetched_l1_block(10, vec![unsupported_commit.clone(), valid_commit.clone()]),
            make_fetched_l1_block(11, vec![valid_reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert_eq!(outcome.envelopes().len(), 1);
        assert_eq!(
            outcome.envelopes()[0].commit_txid(),
            valid_commit.compute_txid()
        );
        assert_eq!(outcome.quarantined().len(), 1);
        assert_eq!(
            outcome.quarantined()[0].commit_txid(),
            unsupported_commit.compute_txid()
        );
        assert!(matches!(
            outcome.quarantined()[0].reason(),
            QuarantineReason::UnsupportedVersion { version: 1 }
        ));
    }

    #[test]
    fn test_malformed_marker_isolated() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let mut malformed_commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        malformed_commit.output[0].script_pubkey = build_commit_marker_with_extra_opcode();
        let valid_commit = make_distinct_commit_tx();
        let valid_reveal =
            build_reveal_tx(valid_commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let blocks = vec![
            make_fetched_l1_block(10, vec![malformed_commit.clone(), valid_commit.clone()]),
            make_fetched_l1_block(11, vec![valid_reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert_eq!(outcome.envelopes().len(), 1);
        assert_eq!(
            outcome.envelopes()[0].commit_txid(),
            valid_commit.compute_txid()
        );
        assert_eq!(outcome.quarantined().len(), 1);
        assert_eq!(
            outcome.quarantined()[0].commit_txid(),
            malformed_commit.compute_txid()
        );
        assert!(matches!(
            outcome.quarantined()[0].reason(),
            QuarantineReason::MalformedMarker { .. }
        ));
    }

    #[test]
    fn test_wrong_key_candidate_quarantined() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let non_sequencer_pubkey = make_non_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, non_sequencer_pubkey, b"chunk");
        let blocks = vec![
            make_fetched_l1_block(10, vec![commit.clone()]),
            make_fetched_l1_block(11, vec![reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
        assert_eq!(outcome.quarantined().len(), 1);
        assert_eq!(
            outcome.quarantined()[0].commit_txid(),
            commit.compute_txid()
        );
        assert!(matches!(
            outcome.quarantined()[0].reason(),
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_non_tapscript_leaf_quarantined() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let reveal = build_reveal_with_leaf_version(
            build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk"),
            make_future_leaf_version(),
        );
        let blocks = vec![
            make_fetched_l1_block(10, vec![commit.clone()]),
            make_fetched_l1_block(11, vec![reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
        assert_eq!(outcome.quarantined().len(), 1);
        assert_eq!(
            outcome.quarantined()[0].commit_txid(),
            commit.compute_txid()
        );
        assert!(matches!(
            outcome.quarantined()[0].reason(),
            QuarantineReason::UnauthenticatedReveal { .. }
        ));
    }

    #[test]
    fn test_no_slot_commit_reported() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 0, false);
        let blocks = vec![make_fetched_l1_block(10, vec![commit.clone()])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
        assert_eq!(outcome.quarantined().len(), 1);
        assert_eq!(
            outcome.quarantined()[0].commit_txid(),
            commit.compute_txid()
        );
        assert!(matches!(
            outcome.quarantined()[0].reason(),
            QuarantineReason::MissingRevealSlots
        ));
    }

    #[test]
    fn test_missing_marker_ignored() {
        let mut commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        commit.output.swap(0, 1);

        assert!(matches!(
            classify_transaction(&commit, make_alpen_magic_bytes()),
            TxClassification::Unrelated
        ));
    }

    #[test]
    fn test_no_slot_commit_quarantined() {
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 0, false);

        let TxClassification::QuarantinedCommit { reason } =
            classify_transaction(&commit, make_alpen_magic_bytes())
        else {
            panic!("commit without reveal slots must be quarantined");
        };
        assert!(matches!(reason, QuarantineReason::MissingRevealSlots));
    }

    #[test]
    fn test_multi_slot_reveal_rejected() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 2, false);
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
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let err = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect_err("one reveal spending multiple slots must fail");

        assert!(matches!(
            err,
            QuarantineReason::MalformedEnvelope {
                source: DaParseError::RevealMultipleCommitSpends,
            }
        ));
    }

    #[test]
    fn test_ambiguous_p2tr_change_quarantined() {
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, true);

        let TxClassification::QuarantinedCommit { reason } =
            classify_transaction(&commit, make_alpen_magic_bytes())
        else {
            panic!("ambiguous P2TR change must quarantine the commit");
        };
        assert!(matches!(
            reason,
            QuarantineReason::MalformedEnvelope {
                source: DaParseError::AmbiguousTaprootChangeOutput { .. },
            }
        ));
    }

    #[test]
    fn test_cross_block_envelope() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let blocks = vec![
            make_fetched_l1_block(10, vec![commit.clone()]),
            make_fetched_l1_block(11, vec![reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        let envelopes = outcome.envelopes();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit.compute_txid());
        assert_eq!(envelopes[0].chunks(), vec![b"chunk".to_vec()]);
        assert!(outcome.quarantined().is_empty());
    }

    #[test]
    fn test_envelope_txid_order() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit0 = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let commit1 = make_distinct_commit_tx();
        let reveal0 = build_reveal_tx(commit0.compute_txid(), 1, sequencer_pubkey, b"chunk-0");
        let reveal1 = build_reveal_tx(commit1.compute_txid(), 1, sequencer_pubkey, b"chunk-1");
        let blocks = vec![
            make_fetched_l1_block(10, vec![commit0.clone(), commit1.clone()]),
            make_fetched_l1_block(11, vec![reveal0, reveal1]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");
        let envelopes = outcome.envelopes();
        let commit_txids = envelopes
            .iter()
            .map(ParsedEnvelope::commit_txid)
            .collect::<Vec<_>>();

        assert_eq!(envelopes.len(), 2);
        assert!(outcome.quarantined().is_empty());
        assert_eq!(
            commit_txids[0],
            commit0.compute_txid().min(commit1.compute_txid())
        );
        assert_eq!(
            commit_txids[1],
            commit0.compute_txid().max(commit1.compute_txid())
        );
    }

    #[test]
    fn test_missing_reveal_quarantined() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let blocks = vec![make_fetched_l1_block(10, vec![commit])];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
        assert_eq!(outcome.quarantined().len(), 1);
        assert!(matches!(
            outcome.quarantined()[0].reason(),
            QuarantineReason::MissingReveals {
                expected_slots: 1,
                covered_slots: 0,
            }
        ));
    }

    #[test]
    fn test_cross_commit_reveal_quarantined() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit0 = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let commit1 = make_distinct_commit_tx();
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
            make_fetched_l1_block(10, vec![commit0, commit1]),
            make_fetched_l1_block(11, vec![reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert!(outcome.envelopes().is_empty());
        assert_eq!(outcome.quarantined().len(), 2);
        assert!(outcome.quarantined().iter().all(|candidate| matches!(
            candidate.reason(),
            QuarantineReason::RevealCrossesCommits { .. }
        )));
    }

    #[test]
    fn test_malformed_envelope_isolated() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let malformed_commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let valid_commit = make_distinct_commit_tx();
        let malformed_reveal0 = build_reveal_tx(
            malformed_commit.compute_txid(),
            1,
            sequencer_pubkey,
            b"chunk-0",
        );
        let mut malformed_reveal1 = build_reveal_tx(
            malformed_commit.compute_txid(),
            1,
            sequencer_pubkey,
            b"chunk-1",
        );
        malformed_reveal1.input[0].sequence = Sequence::MAX;
        let valid_reveal =
            build_reveal_tx(valid_commit.compute_txid(), 1, sequencer_pubkey, b"valid");
        let blocks = vec![
            make_fetched_l1_block(10, vec![malformed_commit.clone(), valid_commit.clone()]),
            make_fetched_l1_block(11, vec![malformed_reveal0, malformed_reveal1, valid_reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        assert_eq!(outcome.envelopes().len(), 1);
        assert_eq!(
            outcome.envelopes()[0].commit_txid(),
            valid_commit.compute_txid()
        );
        assert_eq!(outcome.quarantined().len(), 1);
        assert_eq!(
            outcome.quarantined()[0].commit_txid(),
            malformed_commit.compute_txid()
        );
        assert!(matches!(
            outcome.quarantined()[0].reason(),
            QuarantineReason::MalformedEnvelope {
                source: DaParseError::DuplicateReveal { vout: 1 },
            }
        ));
    }

    #[test]
    fn test_duplicate_commit_txid_rejected() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let blocks = vec![
            make_fetched_l1_block(10, vec![commit.clone()]),
            make_fetched_l1_block(11, vec![commit.clone()]),
        ];

        let err = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect_err("duplicate commit txid must fail");

        assert!(matches!(
            err,
            ScanError::DuplicateCommitTxid { txid } if txid == commit.compute_txid()
        ));
    }

    #[test]
    fn test_non_commit_blocks_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
        let reveal = build_reveal_tx(commit.compute_txid(), 1, sequencer_pubkey, b"chunk");
        let reveal_block = Block {
            header: Header {
                version: Version::from_consensus(1),
                prev_blockhash: BlockHash::all_zeros(),
                merkle_root: TxMerkleNode::all_zeros(),
                time: 0,
                bits: CompactTarget::from_consensus(0),
                nonce: 0,
            },
            txdata: vec![reveal],
        };
        let blocks = vec![
            make_fetched_l1_block(10, vec![commit.clone()]),
            L1BlockData::new(11, BlockHash::from_byte_array([11; 32]), reveal_block),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        let envelopes = outcome.envelopes();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit.compute_txid());
        assert!(outcome.quarantined().is_empty());
    }

    #[test]
    fn test_change_spend_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
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
                script_pubkey: ScriptBuf::new_p2wpkh(&WPubkeyHash::all_zeros()),
            }],
        };
        let blocks = vec![
            make_fetched_l1_block(10, vec![commit.clone()]),
            make_fetched_l1_block(11, vec![change_spend, reveal]),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        let envelopes = outcome.envelopes();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), commit_txid);
        assert_eq!(envelopes[0].chunks(), vec![b"chunk".to_vec()]);
        assert!(outcome.quarantined().is_empty());
    }

    #[test]
    fn test_non_slot_input_ignored() {
        let sequencer_pubkey = make_sequencer_pubkey();
        let commit = build_commit_tx(make_alpen_magic_bytes(), 0, 1, false);
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
        let commit = make_commit_candidate(commit);
        let reveals = make_reveal_candidates(vec![reveal]);
        let reveal_refs = build_reveal_refs(&reveals);
        let parsed = authenticate_and_parse_envelope(&commit, &reveal_refs, sequencer_pubkey)
            .expect("non-slot input does not require DA reveal authentication");

        assert_eq!(parsed.commit_txid(), commit_txid);
        assert_eq!(parsed.chunks(), vec![b"chunk".to_vec()]);
    }

    #[test]
    fn test_btcio_writer_roundtrip() {
        let keypair = make_sequencer_keypair();
        let sequencer_pubkey = XOnlyPublicKey::from_keypair(&keypair).0;
        let change_address = make_writer_change_address();
        let config = EnvelopeConfig::new(
            make_alpen_magic_bytes(),
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
            &make_alpen_magic_bytes(),
            DA_BLOB_VERSION,
            &keypair,
            vec![make_writer_funding_utxo(&change_address)],
        )
        .expect("btcio writer builds envelope");
        let blocks = vec![
            make_fetched_l1_block(10, vec![txs.commit_tx.clone()]),
            make_fetched_l1_block(11, txs.reveal_txs),
        ];

        let outcome = scan_preloaded_l1_blocks(&blocks, make_alpen_magic_bytes(), sequencer_pubkey)
            .expect("scan succeeds");

        let envelopes = outcome.envelopes();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].commit_txid(), txs.commit_tx.compute_txid());
        assert_eq!(envelopes[0].chunks(), chunks);
        assert!(outcome.quarantined().is_empty());
    }
}
