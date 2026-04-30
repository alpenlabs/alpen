//! Test utilities for the OL STF implementation.

#![allow(unreachable_pub, reason = "test util module")]

use std::mem;

use ssz_primitives::FixedBytes;
use strata_acct_types::{
    AccountId, AccumulatorClaim, BitcoinAmount, Hash, MessageEntry, Mmr64, MsgPayload,
    RawMerkleProof, SentMessage, SentTransfer, StrataHasher, TxEffects, tree_hash::TreeHash,
};
use strata_asm_common::AsmManifest;
use strata_identifiers::{
    AccountSerial, Buf32, Epoch, L1BlockCommitment, L1BlockId, Slot, WtxidsRoot,
};
use strata_ledger_types::*;
use strata_merkle::{CompactMmr64, MerkleProof, Mmr};
use strata_ol_chain_types_new::*;
use strata_ol_params::OLParams;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{OLAccountState, OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;

/// Creates a genesis state layer using minimal empty parameters.
pub fn create_test_genesis_state() -> MemoryStateBaseLayer {
    let params = OLParams::new_empty(L1BlockCommitment::default());
    let state = OLState::from_genesis_params(&params).expect("valid params");
    MemoryStateBaseLayer::new(state)
}

use crate::{
    ExecResult,
    assembly::{
        BlockComponents, CompletedBlock, ConstructBlockOutput, construct_block,
        execute_and_complete_block,
    },
    context::{BlockContext, BlockInfo},
    errors::ExecError,
    verification::verify_block,
};

/// Execute a block with the given block info and return the completed block.
pub fn execute_block(
    state: &mut MemoryStateBaseLayer,
    block_info: &BlockInfo,
    parent_header: Option<&OLBlockHeader>,
    components: BlockComponents,
) -> ExecResult<CompletedBlock> {
    let block_context = BlockContext::new(block_info, parent_header);
    execute_and_complete_block(state, block_context, components)
}

/// Execute a block and return the construct output which includes both the completed block and
/// execution outputs. This is useful for tests that need to inspect the logs.
pub fn execute_block_with_outputs(
    state: &mut MemoryStateBaseLayer,
    block_info: &BlockInfo,
    parent_header: Option<&OLBlockHeader>,
    components: BlockComponents,
) -> ExecResult<ConstructBlockOutput> {
    let block_context = BlockContext::new(block_info, parent_header);
    construct_block(state, block_context, components)
}

/// Build and execute a chain of empty blocks starting from genesis.
///
/// Returns the headers of all blocks in the chain.
pub fn build_empty_chain(
    state: &mut MemoryStateBaseLayer,
    num_blocks: usize,
    slots_per_epoch: u64,
) -> ExecResult<Vec<CompletedBlock>> {
    let mut blocks = Vec::with_capacity(num_blocks);

    if num_blocks == 0 {
        return Ok(blocks);
    }

    // Execute genesis block (always terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_manifest = AsmManifest::new(
        1, // Genesis manifest should be at height 1 when last_l1_height is 0
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    )
    .expect("genesis manifest should be valid");
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis = execute_block(state, &genesis_info, None, genesis_components)?;
    blocks.push(genesis);

    // Execute subsequent blocks
    for i in 1..num_blocks {
        let slot = i as u64;
        // With genesis as terminal: epoch 0 is just genesis, then normal epochs
        let epoch = ((slot - 1) / slots_per_epoch + 1) as u32;
        let parent = blocks[i - 1].header();
        let timestamp = 1000000 + (i as u64 * 1000);
        let block_info = BlockInfo::new(timestamp, slot, epoch);

        // Check if this should be a terminal block
        // After genesis, terminal blocks are at slots that are multiples of slots_per_epoch
        let is_terminal = slot.is_multiple_of(slots_per_epoch);

        let components = if is_terminal {
            // Create a terminal block with a dummy manifest
            let dummy_manifest = AsmManifest::new(
                state.last_l1_height() + 1, // Next L1 height after state's last seen
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            )
            .expect("dummy manifest should be valid");
            BlockComponents::new_manifests(vec![dummy_manifest])
        } else {
            BlockComponents::new_empty()
        };

        let block = execute_block(state, &block_info, Some(parent), components)?;
        blocks.push(block);
    }

    Ok(blocks)
}

/// Build and execute a chain of empty blocks starting from genesis.
///
/// Returns the headers of all blocks in the chain.
pub fn build_empty_chain_headers(
    state: &mut MemoryStateBaseLayer,
    num_blocks: usize,
    slots_per_epoch: u64,
) -> ExecResult<Vec<OLBlockHeader>> {
    Ok(build_empty_chain(state, num_blocks, slots_per_epoch)?
        .into_iter()
        .map(|b| b.into_header())
        .collect())
}

/// Creates a snark account with initial balance in the given state.
fn create_snark_account(state: &mut MemoryStateBaseLayer) {
    let snark_id = get_test_snark_account_id();
    let update_vk = PredicateKey::always_accept();
    let initial_state_root = get_test_state_root(1);
    let balance = BitcoinAmount::from_sat(100_000_000);
    let new_acct_data = NewAccountData::new(
        balance,
        NewAccountTypeState::Snark {
            update_vk,
            initial_state_root,
        },
    );
    state
        .create_new_account(snark_id, new_acct_data)
        .expect("should create snark account");
}

/// Builds a chain of blocks with a mix of transaction types.
///
/// Uses a 4-block cycle after genesis:
/// - `i % 4 == 1`: GAM to snark account (populates inbox for later processing)
/// - `i % 4 == 2`: GAM to regular target
/// - `i % 4 == 3`: Complex SnarkAccountUpdate (processes inbox messages with MMR proofs, includes
///   output transfers)
/// - `i % 4 == 0`: Empty block
///
/// The last slot must equal `slots_per_epoch` to produce a terminal block with manifest processing.
pub fn build_chain_with_transactions(
    state: &mut MemoryStateBaseLayer,
    num_blocks: usize,
    slots_per_epoch: u64,
) -> Vec<CompletedBlock> {
    let mut blocks = Vec::with_capacity(num_blocks);

    let gam_target = test_account_id(1);
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Create accounts before genesis
    create_snark_account(state);
    create_empty_account(state, gam_target);
    create_empty_account(state, recipient_id);

    // Terminal genesis (with manifest) so epoch advances from 0 to 1
    let genesis_manifest = AsmManifest::new(
        1,
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    )
    .expect("genesis manifest should be valid");
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis =
        execute_block(state, &genesis_info, None, genesis_components).expect("genesis should work");
    blocks.push(genesis);

    let mut state_root_counter: u8 = 2;
    let mut inbox_tracker = InboxMmrTracker::new();
    let mut pending_msgs: Vec<MessageEntry> = Vec::new();
    let mut pending_proofs: Vec<RawMerkleProof> = Vec::new();

    for i in 1..num_blocks {
        let slot = i as u64;
        let epoch = ((slot - 1) / slots_per_epoch + 1) as u32;
        let parent = blocks[i - 1].header();
        let timestamp = 1_000_000 + (i as u64 * 1000);
        let block_info = BlockInfo::new(timestamp, slot, epoch);

        let is_terminal = slot.is_multiple_of(slots_per_epoch);

        let components = if is_terminal {
            let dummy_manifest = AsmManifest::new(
                state.last_l1_height() + 1,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            )
            .expect("dummy manifest should be valid");
            BlockComponents::new(
                OLTxSegment::new(vec![make_gam_tx(gam_target)])
                    .expect("tx segment should be within limits"),
                Some(
                    OLL1ManifestContainer::new(vec![dummy_manifest])
                        .expect("single manifest should succeed"),
                ),
            )
        } else if i % 4 == 1 {
            // GAM to snark account: populates the snark's inbox for later processing
            let msg_data = format!("inbox msg at slot {i}").into_bytes();

            let msg_entry = MessageEntry::new(
                crate::SEQUENCER_ACCT_ID,
                epoch,
                MsgPayload::new(BitcoinAmount::from_sat(0), msg_data.clone()),
            );
            let proof = inbox_tracker.add_message(&msg_entry);
            pending_msgs.push(msg_entry);
            pending_proofs.push(proof);

            let gam_tx = OLTransaction::new(
                OLTransactionData::new_gam(snark_id, msg_data),
                TxProofs::new_empty(),
            );
            BlockComponents::new_txs_from_ol_transactions(vec![gam_tx])
        } else if i % 4 == 3 && !pending_msgs.is_empty() {
            // Complex SnarkAccountUpdate: processes inbox messages with valid MMR proofs
            // and transfers funds to the recipient account
            let (_, snark_state) = get_snark_state_expect(state, snark_id);
            let builder = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
                .with_processed_msgs(mem::take(&mut pending_msgs))
                .with_inbox_proofs(mem::take(&mut pending_proofs))
                .with_transfer(recipient_id, 1_000_000);
            let new_state_root = get_test_state_root(state_root_counter);
            state_root_counter = state_root_counter.wrapping_add(1);
            let tx = builder.build(snark_id, new_state_root, vec![0u8; 32]);
            BlockComponents::new_txs_from_ol_transactions(vec![tx])
        } else if i % 4 == 2 {
            // GAM to regular target account
            let gam_tx = OLTransaction::new(
                OLTransactionData::new_gam(gam_target, vec![]),
                TxProofs::new_empty(),
            );
            BlockComponents::new_txs_from_ol_transactions(vec![gam_tx])
        } else {
            BlockComponents::new_empty()
        };

        let block = execute_block(state, &block_info, Some(parent), components)
            .expect("block execution should succeed");
        blocks.push(block);
    }

    blocks
}

/// Create test account IDs with predictable values.
pub fn test_account_id(index: u32) -> AccountId {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&index.to_le_bytes());
    AccountId::from(bytes)
}

/// Create a test L1 block ID with predictable values.
pub fn test_l1_block_id(index: u32) -> L1BlockId {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&index.to_le_bytes());
    L1BlockId::from(Buf32::from(bytes))
}

/// Assert that a block header matches expected epoch and slot values.
pub fn assert_block_position(header: &OLBlockHeader, expected_epoch: u64, expected_slot: u64) {
    assert_eq!(
        header.epoch() as u64,
        expected_epoch,
        "Block epoch mismatch: expected {}, got {}",
        expected_epoch,
        header.epoch()
    );
    assert_eq!(
        header.slot(),
        expected_slot,
        "Block slot mismatch: expected {}, got {}",
        expected_slot,
        header.slot()
    );
}

/// Assert that the state has been properly updated after block execution.
pub fn assert_state_updated(
    state: &mut MemoryStateBaseLayer,
    expected_epoch: u64,
    expected_slot: u64,
) {
    assert_eq!(
        state.cur_epoch() as u64,
        expected_epoch,
        "test: state epoch mismatch"
    );
    assert_eq!(state.cur_slot(), expected_slot, "test: state slot mismatch");
}

// ===== Verification Test Utilities =====

/// Assert that block verification succeeds.
pub fn assert_verification_succeeds<S: IStateAccessorMut>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
) {
    let result = verify_block(state, header, parent_header.as_ref(), body);
    assert!(
        result.is_ok(),
        "Block verification failed when it should have succeeded: {:?}",
        result.err()
    );
}

/// Assert that block verification fails with a specific error.
pub fn assert_verification_fails_with(
    state: &mut impl IStateAccessorMut,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
    error_matcher: impl Fn(&ExecError) -> bool,
) {
    let result = verify_block(state, header, parent_header.as_ref(), body);
    assert!(
        result.is_err(),
        "Block verification succeeded when it should have failed"
    );

    let err = result.unwrap_err();
    assert!(error_matcher(&err), "Unexpected error type. Got: {:?}", err);
}

/// Create a tampered block header with a different parent block ID.
pub fn tamper_parent_blkid(
    header: &OLBlockHeader,
    new_parent: strata_ol_chain_types_new::OLBlockId,
) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        new_parent,
        *header.body_root(),
        *header.state_root(),
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different state root.
pub fn tamper_state_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        *header.parent_blkid(),
        *header.body_root(),
        new_root,
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different logs root.
pub fn tamper_logs_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        *header.parent_blkid(),
        *header.body_root(),
        *header.state_root(),
        new_root,
    )
}

/// Create a tampered block header with a different body root.
pub fn tamper_body_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        *header.parent_blkid(),
        new_root,
        *header.state_root(),
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different slot.
pub fn tamper_slot(header: &OLBlockHeader, new_slot: u64) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        new_slot,
        header.epoch(),
        *header.parent_blkid(),
        *header.body_root(),
        *header.state_root(),
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different epoch.
pub fn tamper_epoch(header: &OLBlockHeader, new_epoch: u32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        new_epoch,
        *header.parent_blkid(),
        *header.body_root(),
        *header.state_root(),
        *header.logs_root(),
    )
}

// ===== SNARK Account Test Utilities =====

/// Common test account IDs for consistent testing
pub const TEST_SNARK_ACCOUNT_ID: u32 = 100;
pub const TEST_RECIPIENT_ID: u32 = 200;
pub const TEST_NONEXISTENT_ID: u32 = 999;

/// Get the standard test snark account ID
pub fn get_test_snark_account_id() -> AccountId {
    test_account_id(TEST_SNARK_ACCOUNT_ID)
}

/// Get the standard test recipient account ID
pub fn get_test_recipient_account_id() -> AccountId {
    test_account_id(TEST_RECIPIENT_ID)
}

/// Get a test state root with a specific variant
pub fn get_test_state_root(variant: u8) -> Hash {
    Hash::from([variant; 32])
}

/// Get a test proof with a specific variant
pub fn get_test_proof(variant: u8) -> Vec<u8> {
    vec![variant; 100]
}

/// Helper to track inbox MMR proofs in parallel with the actual STF inbox MMR.
#[derive(Debug)]
pub struct InboxMmrTracker {
    mmr: Mmr64,
    proofs: Vec<MerkleProof<[u8; 32]>>,
}

impl Default for InboxMmrTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl InboxMmrTracker {
    pub fn new() -> Self {
        Self {
            mmr: Mmr64::from_generic(&CompactMmr64::new(64)),
            proofs: Vec::new(),
        }
    }

    /// Adds a message entry to the tracker and returns a raw merkle proof for it.
    pub fn add_message(&mut self, entry: &MessageEntry) -> RawMerkleProof {
        let hash = <MessageEntry as TreeHash>::tree_hash_root(entry);

        let proof = Mmr::<StrataHasher>::add_leaf_updating_proof_list(
            &mut self.mmr,
            hash.into_inner(),
            &mut self.proofs,
        )
        .expect("mmr: can't add leaf");

        self.proofs.push(proof.clone());

        // Convert MerkleProof to RawMerkleProof (strip the index)
        RawMerkleProof {
            cohashes: proof
                .cohashes()
                .iter()
                .map(|h| FixedBytes::from(*h))
                .collect::<Vec<_>>()
                .try_into()
                .expect("proof cohashes should fit into RawMerkleProof"),
        }
    }

    /// Returns the number of entries in the tracked MMR
    pub fn num_entries(&self) -> u64 {
        self.mmr.num_entries()
    }
}

/// Tracks ASM manifests in a parallel MMR to generate proofs for ledger references.
#[derive(Debug)]
pub struct ManifestMmrTracker {
    mmr: Mmr64,
    proofs: Vec<MerkleProof<[u8; 32]>>,
}

impl Default for ManifestMmrTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ManifestMmrTracker {
    pub fn new() -> Self {
        Self {
            mmr: Mmr64::from_generic(&CompactMmr64::new(64)),
            proofs: Vec::new(),
        }
    }

    /// Adds a manifest to the tracker and returns a RawMerkleProof for it.
    pub fn add_manifest(&mut self, manifest: &AsmManifest) -> (u64, RawMerkleProof) {
        let hash = <AsmManifest as TreeHash>::tree_hash_root(manifest);
        let index = self.mmr.num_entries();

        let proof = Mmr::<StrataHasher>::add_leaf_updating_proof_list(
            &mut self.mmr,
            hash.into_inner(),
            &mut self.proofs,
        )
        .expect("mmr: can't add leaf");

        self.proofs.push(proof.clone());

        let raw_proof = RawMerkleProof {
            cohashes: proof
                .cohashes()
                .iter()
                .map(|h| FixedBytes::from(*h))
                .collect::<Vec<_>>()
                .try_into()
                .expect("proof cohashes should fit into RawMerkleProof"),
        };

        (index, raw_proof)
    }

    /// Returns the number of manifests in the tracked MMR
    pub fn num_entries(&self) -> u64 {
        self.mmr.num_entries()
    }
}

/// Creates a SNARK account with initial balance and executes an empty genesis block.
/// Returns the completed genesis block.
pub fn setup_genesis_with_snark_account(
    state: &mut MemoryStateBaseLayer,
    snark_id: AccountId,
    initial_balance: u64,
) -> CompletedBlock {
    let update_vk = PredicateKey::always_accept();
    let initial_state_root = get_test_state_root(1);
    let balance = BitcoinAmount::from_sat(initial_balance);
    let new_acct_data = NewAccountData::new(
        balance,
        NewAccountTypeState::Snark {
            update_vk,
            initial_state_root,
        },
    );
    state
        .create_new_account(snark_id, new_acct_data)
        .expect("Should create snark account");

    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_components = BlockComponents::new_empty();
    execute_block(state, &genesis_info, None, genesis_components).expect("Genesis should execute")
}

/// Helper to create additional empty accounts (for testing transfers/messages)
pub fn create_empty_account(
    state: &mut MemoryStateBaseLayer,
    account_id: AccountId,
) -> AccountSerial {
    let new_acct_data = NewAccountData::new_empty(NewAccountTypeState::Empty);
    state
        .create_new_account(account_id, new_acct_data)
        .expect("Should create empty account")
}

/// Helper to make a GAM transaction targeting the given account with empty payload data.
pub fn make_gam_tx(dest: AccountId) -> OLTransaction {
    OLTransaction::new(
        OLTransactionData::new_gam(dest, vec![]),
        TxProofs::new_empty(),
    )
}

/// Helper to execute a transaction in a non-genesis block.
///
/// Accepts an `OLTransaction` directly.
pub fn execute_tx_in_block(
    state: &mut MemoryStateBaseLayer,
    parent_header: &OLBlockHeader,
    tx: OLTransaction,
    slot: Slot,
    epoch: Epoch,
) -> ExecResult<CompletedBlock> {
    let block_info = BlockInfo::new(1_001_000, slot, epoch);
    let components = BlockComponents::new_txs_from_ol_transactions(vec![tx]);
    execute_block(state, &block_info, Some(parent_header), components)
}

/// Builder pattern for creating SnarkAccountUpdate transactions.
/// Captures the starting state and builds toward the resulting state,
/// ensuring correct sequence numbers and message indices.
#[derive(Debug)]
pub struct SnarkUpdateBuilder {
    // Captured from old state at construction
    seq_no: u64,
    old_msg_idx: u64,

    // Built up via with_* methods
    processed_messages: Vec<MessageEntry>,
    inbox_proofs: Vec<RawMerkleProof>,
    effects: TxEffects,
    ledger_ref_claims: Vec<AccumulatorClaim>,
    ledger_ref_proofs: Vec<RawMerkleProof>,
}

impl SnarkUpdateBuilder {
    /// Create builder from current account state (captures starting point)
    pub fn from_snark_state(snark_state: OLSnarkAccountState) -> Self {
        Self {
            seq_no: *snark_state.seqno().inner(),
            old_msg_idx: snark_state.next_inbox_msg_idx(),
            processed_messages: vec![],
            inbox_proofs: vec![],
            effects: TxEffects::default(),
            ledger_ref_claims: vec![],
            ledger_ref_proofs: vec![],
        }
    }

    /// Add processed messages
    pub fn with_processed_msgs(mut self, messages: Vec<MessageEntry>) -> Self {
        self.processed_messages = messages;
        self
    }

    /// Add inbox proofs for the processed messages
    pub fn with_inbox_proofs(mut self, proofs: Vec<RawMerkleProof>) -> Self {
        self.inbox_proofs = proofs;
        self
    }

    /// Add a single transfer effect
    pub fn with_transfer(mut self, dest: AccountId, amount: u64) -> Self {
        self.effects
            .add_transfer(SentTransfer::new(dest, BitcoinAmount::from_sat(amount)));
        self
    }

    /// Add a single message effect
    pub fn with_output_message(mut self, dest: AccountId, amount: u64, data: Vec<u8>) -> Self {
        let payload = MsgPayload::new(BitcoinAmount::from_sat(amount), data);
        self.effects.add_message(SentMessage::new(dest, payload));
        self
    }

    /// Set ledger reference claims and proofs
    pub fn with_ledger_refs(
        mut self,
        claims: Vec<AccumulatorClaim>,
        proofs: Vec<RawMerkleProof>,
    ) -> Self {
        self.ledger_ref_claims = claims;
        self.ledger_ref_proofs = proofs;
        self
    }

    /// Build the full OLTransaction with the resulting state root.
    pub fn build(self, acct_id: AccountId, new_state_root: Hash, proof: Vec<u8>) -> OLTransaction {
        // Calculate new message index based on messages processed
        let new_msg_idx = self.old_msg_idx + self.processed_messages.len() as u64;

        // Build SauTxPayload
        let proof_state = SauTxProofState {
            new_next_msg_idx: new_msg_idx,
            inner_state_root: <[u8; 32]>::from(new_state_root).into(),
        };
        let update_data = SauTxUpdateData {
            seq_no: self.seq_no,
            proof_state,
            extra_data: vec![].try_into().unwrap(),
        };

        // Build ledger refs
        let ledger_refs = if self.ledger_ref_claims.is_empty() {
            SauTxLedgerRefs::new_empty()
        } else {
            let claim_list =
                ClaimList::new(self.ledger_ref_claims).expect("test: too many ledger ref claims");
            SauTxLedgerRefs::new_with_claims(claim_list)
        };

        let operation_data = SauTxOperationData {
            update_data,
            messages: self.processed_messages.try_into().unwrap(),
            ledger_refs,
        };

        let sau_payload = SauTxPayload {
            target: acct_id,
            operation_data,
        };
        let payload = TransactionPayload::SnarkAccountUpdate(sau_payload);

        // Build TxProofs
        let mut all_accumulator_proofs = Vec::new();
        // Inbox proofs come first, then ledger ref proofs
        all_accumulator_proofs.extend(self.inbox_proofs);
        all_accumulator_proofs.extend(self.ledger_ref_proofs);

        let accumulator_proofs = if all_accumulator_proofs.is_empty() {
            None
        } else {
            Some(RawMerkleProofList {
                proofs: all_accumulator_proofs
                    .try_into()
                    .expect("test: too many accumulator proofs"),
            })
        };

        let predicate_satisfiers = if proof.is_empty() {
            None
        } else {
            Some(ProofSatisfierList {
                proofs: vec![ProofSatisfier {
                    proof: proof
                        .try_into()
                        .expect("test: proof should fit in ProofSatisfier"),
                }]
                .try_into()
                .expect("test: too many predicate proofs"),
            })
        };

        let tx_proofs = TxProofs::new(predicate_satisfiers, accumulator_proofs);

        let data = OLTransactionData {
            payload,
            constraints: TxConstraints::default(),
            effects: self.effects,
        };

        OLTransaction::new(data, tx_proofs)
    }
}

/// Helper to get snark account state from a state accessor, panicking if not found or not a snark
/// account.
pub fn get_snark_state_expect(
    state: &MemoryStateBaseLayer,
    snark_id: AccountId,
) -> (&OLAccountState, &OLSnarkAccountState) {
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    (snark_account, snark_account.as_snark_account().unwrap())
}

/// Helper for creating invalid snark updates for error testing.
/// This bypasses the builder's correctness guarantees.
pub fn create_unchecked_snark_update(
    target: AccountId,
    wrong_seq_no: u64,
    new_state_root: Hash,
    new_msg_idx: u64,
    effects: TxEffects,
) -> OLTransaction {
    let proof_state = SauTxProofState {
        new_next_msg_idx: new_msg_idx,
        inner_state_root: <[u8; 32]>::from(new_state_root).into(),
    };
    let update_data = SauTxUpdateData {
        seq_no: wrong_seq_no,
        proof_state,
        extra_data: vec![].try_into().unwrap(),
    };

    let operation_data = SauTxOperationData {
        update_data,
        messages: vec![].try_into().unwrap(),
        ledger_refs: SauTxLedgerRefs::new_empty(),
    };

    let sau_payload = SauTxPayload {
        target,
        operation_data,
    };
    let payload = TransactionPayload::SnarkAccountUpdate(sau_payload);

    let data = OLTransactionData {
        payload,
        constraints: TxConstraints::default(),
        effects,
    };

    // Dummy proof
    let tx_proofs = TxProofs::new(
        Some(ProofSatisfierList {
            proofs: vec![ProofSatisfier {
                proof: vec![0u8; 32].try_into().unwrap(),
            }]
            .try_into()
            .expect("test: too many predicate proofs"),
        }),
        None,
    );

    OLTransaction::new(data, tx_proofs)
}
