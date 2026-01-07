//! Test utilities for the OL STF implementation.

#![allow(unreachable_pub, reason = "test util module")]

use ssz_primitives::FixedBytes;
use strata_acct_types::{
    AccountId, BitcoinAmount, Hash, Mmr64, RawMerkleProof, StrataHasher, tree_hash::TreeHash,
};
use strata_asm_common::AsmManifest;
use strata_identifiers::{AccountSerial, Buf32, Epoch, L1BlockId, Slot, WtxidsRoot};
use strata_ledger_types::{
    AccountTypeState, IAccountState, ISnarkAccountState, IStateAccessor, NewAccountData,
};
use strata_merkle::{CompactMmr64, MerkleProof, Mmr};
use strata_ol_chain_types_new::{OLBlockHeader, SnarkAccountUpdateTxPayload, TransactionPayload};
use strata_ol_state_types::{NativeSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{
    LedgerRefProofs, LedgerRefs, MessageEntry, MessageEntryProof, OutputMessage, OutputTransfer,
    ProofState, SnarkAccountUpdate, SnarkAccountUpdateContainer, UpdateAccumulatorProofs,
    UpdateOperationData, UpdateOutputs,
};

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
    state: &mut OLState,
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
    state: &mut OLState,
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
    state: &mut OLState,
    num_blocks: usize,
    slots_per_epoch: u64,
) -> ExecResult<Vec<OLBlockHeader>> {
    let mut headers = Vec::with_capacity(num_blocks);

    if num_blocks == 0 {
        return Ok(headers);
    }

    // Execute genesis block (always terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_manifest = AsmManifest::new(
        0,
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    );
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis = execute_block(state, &genesis_info, None, genesis_components)?;
    headers.push(genesis.header().clone());

    // Execute subsequent blocks
    for i in 1..num_blocks {
        let slot = i as u64;
        // With genesis as terminal: epoch 0 is just genesis, then normal epochs
        let epoch = ((slot - 1) / slots_per_epoch + 1) as u32;
        let parent = &headers[i - 1];
        let timestamp = 1000000 + (i as u64 * 1000);
        let block_info = BlockInfo::new(timestamp, slot, epoch);

        // Check if this should be a terminal block
        // After genesis, terminal blocks are at slots that are multiples of slots_per_epoch
        let is_terminal = slot.is_multiple_of(slots_per_epoch);

        let components = if is_terminal {
            // Create a terminal block with a dummy manifest
            let dummy_manifest = AsmManifest::new(
                0,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            );
            BlockComponents::new_manifests(vec![dummy_manifest])
        } else {
            BlockComponents::new_empty()
        };

        let block = execute_block(state, &block_info, Some(parent), components)?;
        headers.push(block.header().clone());
    }

    Ok(headers)
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
pub fn assert_state_updated(state: &mut OLState, expected_epoch: u64, expected_slot: u64) {
    assert_eq!(
        state.cur_epoch() as u64,
        expected_epoch,
        "test: state epoch mismatch"
    );
    assert_eq!(state.cur_slot(), expected_slot, "test: state slot mismatch");
}

// ===== Verification Test Utilities =====

/// Assert that block verification succeeds.
pub fn assert_verification_succeeds<S: IStateAccessor>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
) {
    let result = verify_block(state, header, parent_header, body);
    assert!(
        result.is_ok(),
        "Block verification failed when it should have succeeded: {:?}",
        result.err()
    );
}

/// Assert that block verification fails with a specific error.
pub fn assert_verification_fails_with(
    state: &mut impl IStateAccessor,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
    error_matcher: impl Fn(&ExecError) -> bool,
) {
    let result = verify_block(state, header, parent_header, body);
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
    // We need to create a new header with the modified parent
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
pub fn test_snark_account_id() -> AccountId {
    test_account_id(TEST_SNARK_ACCOUNT_ID)
}

/// Get the standard test recipient account ID
pub fn test_recipient_account_id() -> AccountId {
    test_account_id(TEST_RECIPIENT_ID)
}

/// Get a test state root with a specific variant
pub fn test_state_root(variant: u8) -> Hash {
    Hash::from([variant; 32])
}

/// Helper to track inbox MMR proofs in parallel with the actual STF inbox MMR.
/// This allows generating valid MMR proofs for testing by maintaining proofs as leaves are added.
pub struct InboxMmrTracker {
    mmr: Mmr64,
    proofs: Vec<MerkleProof<[u8; 32]>>,
}

impl InboxMmrTracker {
    pub fn new() -> Self {
        Self {
            mmr: Mmr64::from_generic(&CompactMmr64::new(64)),
            proofs: Vec::new(),
        }
    }

    /// Adds a message entry to the tracker and returns a proof for it.
    /// Uses TreeHash for consistent hashing with insertion and verification.
    pub fn add_message(&mut self, entry: &MessageEntry) -> MessageEntryProof {
        // Compute hash using TreeHash, matching both insertion and verification
        let hash = <MessageEntry as TreeHash>::tree_hash_root(entry);

        // Add to MMR with proof tracking
        let proof = Mmr::<StrataHasher>::add_leaf_updating_proof_list(
            &mut self.mmr,
            hash.into_inner(),
            &mut self.proofs,
        )
        .expect("mmr: can't add leaf");

        self.proofs.push(proof.clone());

        // Convert MerkleProof to RawMerkleProof (strip the index)
        let raw_proof = RawMerkleProof {
            cohashes: proof
                .cohashes()
                .iter()
                .map(|h| FixedBytes::from(*h))
                .collect::<Vec<_>>()
                .into(),
        };

        MessageEntryProof::new(entry.clone(), raw_proof)
    }

    /// Returns the number of entries in the tracked MMR
    pub fn num_entries(&self) -> u64 {
        self.mmr.num_entries()
    }
}

/// Creates a SNARK account with initial balance and executes an empty genesis block.
/// Returns the completed genesis block.
pub fn setup_genesis_with_snark_account(
    state: &mut OLState,
    snark_id: AccountId,
    initial_balance: u64,
) -> CompletedBlock {
    // Create snark account with initial balance directly
    let vk = PredicateKey::always_accept();
    let initial_state_root = test_state_root(1);
    let snark_state = NativeSnarkAccountState::new_fresh(vk, initial_state_root);
    let balance = BitcoinAmount::from_sat(initial_balance);
    let new_acct_data = NewAccountData::new(balance, AccountTypeState::Snark(snark_state));
    state
        .create_new_account(snark_id, new_acct_data)
        .expect("Should create snark account");

    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_components = BlockComponents::new_empty();
    execute_block(state, &genesis_info, None, genesis_components).expect("Genesis should execute")
}

/// Helper to create additional empty accounts (for testing transfers/messages)
pub fn create_empty_account(state: &mut OLState, account_id: AccountId) -> AccountSerial {
    let empty_state = AccountTypeState::Empty;
    let new_acct_data = NewAccountData::new_empty(empty_state);
    state
        .create_new_account(account_id, new_acct_data)
        .expect("Should create empty account")
}

/// Helper to execute a transaction in a non-genesis block
pub fn execute_tx_in_block(
    state: &mut OLState,
    parent_header: &OLBlockHeader,
    tx: TransactionPayload,
    slot: Slot,
    epoch: Epoch,
) -> ExecResult<CompletedBlock> {
    let block_info = BlockInfo::new(1_001_000, slot, epoch);
    let components = BlockComponents::new_txs(vec![tx]);
    execute_block(state, &block_info, Some(parent_header), components)
}

/// Builder pattern for creating SnarkAccountUpdate transactions.
/// Reduces boilerplate when creating updates with various configurations.
pub struct SnarkUpdateBuilder {
    seq_no: u64,
    new_state_root: Hash,
    new_msg_idx: Option<u64>,
    processed_messages: Vec<MessageEntry>,
    inbox_proofs: Vec<MessageEntryProof>,
    outputs: UpdateOutputs,
    ledger_refs: LedgerRefs,
    proof: Vec<u8>,
}

impl SnarkUpdateBuilder {
    /// Create a new builder with required parameters
    pub fn new(seq_no: u64, new_state_root: Hash) -> Self {
        Self {
            seq_no,
            new_state_root,
            new_msg_idx: None,
            processed_messages: vec![],
            inbox_proofs: vec![],
            outputs: UpdateOutputs::new(vec![], vec![]),
            ledger_refs: LedgerRefs::new_empty(),
            proof: vec![0u8; 32], // Default dummy proof
        }
    }

    /// Set the next message index explicitly
    pub fn with_message_index(mut self, idx: u64) -> Self {
        self.new_msg_idx = Some(idx);
        self
    }

    /// Add processed messages
    pub fn with_messages(mut self, messages: Vec<MessageEntry>) -> Self {
        self.processed_messages = messages;
        self
    }

    /// Add inbox proofs for the processed messages
    pub fn with_proofs(mut self, proofs: Vec<MessageEntryProof>) -> Self {
        self.inbox_proofs = proofs;
        self
    }

    /// Set the outputs (transfers and messages)
    pub fn with_outputs(mut self, outputs: UpdateOutputs) -> Self {
        self.outputs = outputs;
        self
    }

    /// Add a single transfer output
    pub fn with_transfer(mut self, dest: AccountId, amount: u64) -> Self {
        let transfer = OutputTransfer::new(dest, BitcoinAmount::from_sat(amount));
        let transfers = vec![transfer];
        let messages = self.outputs.messages().to_vec();
        self.outputs = UpdateOutputs::new(transfers, messages);
        self
    }

    /// Add a single message output
    pub fn with_message(mut self, dest: AccountId, amount: u64, data: Vec<u8>) -> Self {
        let payload = strata_acct_types::MsgPayload::new(BitcoinAmount::from_sat(amount), data);
        let message = OutputMessage::new(dest, payload);
        let transfers = self.outputs.transfers().to_vec();
        let messages = vec![message];
        self.outputs = UpdateOutputs::new(transfers, messages);
        self
    }

    /// Set ledger references
    pub fn with_ledger_refs(mut self, refs: LedgerRefs) -> Self {
        self.ledger_refs = refs;
        self
    }

    /// Set a custom proof (default is dummy proof)
    pub fn with_proof(mut self, proof: Vec<u8>) -> Self {
        self.proof = proof;
        self
    }

    /// Build the transaction, calculating next_msg_idx from processed messages if not set
    pub fn build(self, target: AccountId, state: &OLState) -> TransactionPayload {
        // Calculate next message index if not explicitly set
        let new_msg_idx = if let Some(idx) = self.new_msg_idx {
            idx
        } else {
            // Query current state to get the starting index
            let account = state.get_account_state(target).unwrap().unwrap();
            let snark_state = account.as_snark_account().unwrap();
            let cur_idx = snark_state.next_inbox_msg_idx();
            // Add the number of messages we're processing
            cur_idx + self.processed_messages.len() as u64
        };

        let new_proof_state = ProofState::new(self.new_state_root, new_msg_idx);
        let operation_data = UpdateOperationData::new(
            self.seq_no,
            new_proof_state,
            self.processed_messages,
            self.ledger_refs,
            self.outputs,
            vec![], // extra_data
        );

        let base_update = SnarkAccountUpdate::new(operation_data, self.proof);

        // Build accumulator proofs
        let ledger_ref_proofs = LedgerRefProofs::new(vec![]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(self.inbox_proofs, ledger_ref_proofs);

        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let sau_tx_payload = SnarkAccountUpdateTxPayload::new(target, update_container);

        TransactionPayload::SnarkAccountUpdate(sau_tx_payload)
    }

    /// Build the transaction without querying state (requires setting message index explicitly)
    pub fn build_simple(self, target: AccountId) -> TransactionPayload {
        let new_msg_idx = self
            .new_msg_idx
            .expect("Message index must be set for build_simple");

        let new_proof_state = ProofState::new(self.new_state_root, new_msg_idx);
        let operation_data = UpdateOperationData::new(
            self.seq_no,
            new_proof_state,
            self.processed_messages,
            self.ledger_refs,
            self.outputs,
            vec![], // extra_data
        );

        let base_update = SnarkAccountUpdate::new(operation_data, self.proof);

        let ledger_ref_proofs = LedgerRefProofs::new(vec![]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(self.inbox_proofs, ledger_ref_proofs);

        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let sau_tx_payload = SnarkAccountUpdateTxPayload::new(target, update_container);

        TransactionPayload::SnarkAccountUpdate(sau_tx_payload)
    }
}
