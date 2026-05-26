//! Test utilities for the OL STF implementation.
//!
//! Use [`OLStfFixtureBuilder`] for behavior tests that need protocol-shaped genesis.
//! It seeds accounts and optional ASM manifests, executes terminal genesis, and
//! returns an [`OLStfFixture`].
//!
//! Use [`OLStfFixture::child_block`] for ordinary block execution. The block
//! builder owns parent-header threading, default slot/epoch progression,
//! transaction construction, block execution, and error capture.
//!
//! Use the narrow fixture builder for the behavior under test: SAU builders for
//! snark-account updates, GAM builders for inbox delivery, manifest builders for
//! ASM input, and bridge helpers for bridge log/message encoding. Keep
//! scenario-defining values in the test body: balances, transfer amounts,
//! message payloads, state roots, forced sequence numbers, manifest heights,
//! malformed bytes, and expected errors.
//!
//! Use raw helpers such as [`execute_block`], [`verify_block`],
//! [`execute_tx_in_block`], tamper helpers, and MMR trackers when those
//! low-level mechanics are the behavior under test. The fixture should hide
//! setup plumbing, not the evidence being asserted.
//!
//! Fixture genesis is terminal by default. The first child block defaults to
//! slot `1`, epoch `1`. Snapshot and output helpers reduce lookup boilerplate,
//! but tests should still assert explicitly on balances, seqnos, inbox indexes,
//! logs, roots, staged writes, and error variants.
//!
//! ```ignore
//! let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
//! let recipient_id = make_account_id(TEST_RECIPIENT_ID);
//! let mut fixture = OLStfFixture::builder()
//!     .with_genesis_snark_account(snark_acct_id, |acct| {
//!         acct.with_balance(BitcoinAmount::from_sat(100_000_000))
//!     })
//!     .with_genesis_empty_account(recipient_id)
//!     .execute_genesis();
//!
//! fixture
//!     .child_block()
//!     .with_sau(snark_acct_id, |sau| {
//!         sau.transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
//!     })
//!     .execute();
//!
//! assert_eq!(fixture.account_balance(recipient_id), BitcoinAmount::from_sat(10_000_000));
//! ```
//!
//! For failure-path tests, use `execute_err()`, inspect `err.into_base()`, and
//! pair `fixture.snapshot([...])` with `snapshot.assert_unchanged(&fixture)`.
//! See `sau_validation.rs`.
//!
//! For log inspection, use `execute_with_outputs()` and
//! `output.expect_typed_log::<T>(serial)`. See `sau_logs.rs`.
//!
//! See `verify_header.rs` and `staging_layers.rs` for tests that intentionally
//! bypass the fixture because the low-level STF entrypoint is the behavior under
//! test.

#![allow(unreachable_pub, reason = "test util module")]

use std::{any::type_name, collections::BTreeMap, mem};

use ssz_primitives::FixedBytes;
use ssz_types::VariableList;
use strata_acct_types::{
    AccountId, AccumulatorClaim, BitcoinAmount, Hash, MessageEntry, Mmr64, MsgPayload,
    RawMerkleProof, SentMessage, SentTransfer, StrataHasher, TxEffects, tree_hash::TreeHash,
};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::DepositLog;
use strata_codec::{Codec, VarVec, decode_buf_exact, encode_to_vec};
use strata_identifiers::{
    AccountSerial, Buf32, Epoch, L1BlockCommitment, L1BlockId, L1Height, Slot, SubjectId,
    SubjectIdBytes, WtxidsRoot,
};
use strata_ledger_types::*;
use strata_merkle::{CompactMmr64, MerkleProof, Mmr};
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_chain_types_new::*;
use strata_ol_msg_types::{
    DEFAULT_OPERATOR_FEE, DEPOSIT_MSG_TYPE_ID, DepositMsgData, WITHDRAWAL_MSG_TYPE_ID,
    WithdrawalMsgData,
};
use strata_ol_params::{BridgeParams, OLParams};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{
    MMR_SENTINEL_DUMMY_LEAF, OLAccountState, OLSnarkAccountState, OLState,
};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::Seqno;

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

/// Builds an account ID with predictable bytes.
pub fn make_account_id(index: u32) -> AccountId {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&index.to_le_bytes());
    AccountId::from(bytes)
}

/// Standard numeric account IDs used by tests.
pub const TEST_SNARK_ACCOUNT_ID: u32 = 100;
pub const TEST_RECIPIENT_ID: u32 = 200;
pub const TEST_NONEXISTENT_ID: u32 = 999;

/// Builds a state root with predictable bytes.
pub fn make_state_root(variant: u8) -> Hash {
    Hash::from([variant; 32])
}

/// Builds proof bytes with predictable contents.
pub fn make_proof(variant: u8) -> Vec<u8> {
    vec![variant; 100]
}

/// Builds a genesis state layer using minimal empty parameters.
pub fn make_genesis_state() -> MemoryStateBaseLayer {
    let params = OLParams::new_empty(L1BlockCommitment::default());
    let state = OLState::from_genesis_params(&params).expect("valid params");
    MemoryStateBaseLayer::new(state)
}

/// Builds a GAM transaction targeting the given account with empty payload data.
pub fn make_gam_tx(dest: AccountId) -> OLTransaction {
    OLTransaction::new(
        OLTransactionData::from_gam_bytes(dest, vec![])
            .expect("message payload bytes must fit within SSZ max length"),
        TxProofs::new_empty(),
    )
}

/// Builds an empty ASM manifest with deterministic identifiers.
pub fn make_empty_manifest(height: L1Height, variant: u8) -> AsmManifest {
    FixtureAsmManifestBuilder::new_at_height(height)
        .with_variant(variant)
        .build()
}

/// Builds a manifest containing one valid bridge deposit for `account_serial`.
pub fn make_deposit_manifest_for_account(
    height: L1Height,
    variant: u8,
    account_serial: AccountSerial,
    dest_subject: SubjectId,
    amount: BitcoinAmount,
) -> AsmManifest {
    FixtureAsmManifestBuilder::new_at_height(height)
        .with_variant(variant)
        .with_log(make_deposit_log_for_account(
            account_serial,
            dest_subject,
            amount,
        ))
        .build()
}

/// Builds a bridge deposit log targeting the given account serial and subject.
pub fn make_deposit_log_for_account(
    account_serial: AccountSerial,
    dest_subject: SubjectId,
    amount: BitcoinAmount,
) -> AsmLogEntry {
    let dest_subject_bytes =
        SubjectIdBytes::try_new(dest_subject.inner().to_vec()).expect("valid subject bytes");
    let descriptor = DepositDescriptor::new(account_serial, dest_subject_bytes)
        .expect("valid deposit descriptor");
    let deposit = DepositLog::new(descriptor.encode_to_varvec(), amount.to_sat());
    AsmLogEntry::from_log(&deposit).expect("deposit log should encode")
}

/// Builds a manifest containing one bridge deposit with caller-provided destination bytes.
pub fn make_deposit_manifest_with_destination_bytes(
    height: L1Height,
    variant: u8,
    destination: Vec<u8>,
    amount: BitcoinAmount,
) -> AsmManifest {
    let destination = VarVec::from_vec(destination).expect("destination should fit in VarVec");
    let deposit = DepositLog::new(destination, amount.to_sat());
    let deposit_log = AsmLogEntry::from_log(&deposit).expect("deposit log should encode");
    FixtureAsmManifestBuilder::new_at_height(height)
        .with_variant(variant)
        .with_log(deposit_log)
        .build()
}

/// Builds the inbox message produced by processing a bridge deposit log.
pub fn make_deposit_message_entry(
    epoch: Epoch,
    dest_subject: SubjectId,
    amount: BitcoinAmount,
) -> MessageEntry {
    let deposit_msg = DepositMsgData::new(dest_subject);
    let deposit_body = encode_to_vec(&deposit_msg).expect("deposit message should encode");
    let deposit_data = OwnedMsg::new(DEPOSIT_MSG_TYPE_ID, deposit_body)
        .expect("deposit message should be valid")
        .to_vec();
    MessageEntry::new(
        crate::BRIDGE_GATEWAY_ACCT_ID,
        epoch,
        MsgPayload::from_bytes(amount, deposit_data)
            .expect("message payload bytes must fit within SSZ max length"),
    )
}

/// Builds a bridge withdrawal payload for an output message.
pub fn make_withdrawal_payload(dest_desc: Vec<u8>) -> Vec<u8> {
    let withdrawal = WithdrawalMsgData::new(DEFAULT_OPERATOR_FEE, dest_desc, u32::MAX)
        .expect("withdrawal data should be valid");
    let body = encode_to_vec(&withdrawal).expect("withdrawal message should encode");
    OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, body)
        .expect("withdrawal message should be valid")
        .to_vec()
}

/// Builds terminal genesis components with one empty manifest at L1 height 1.
pub fn build_terminal_genesis_components() -> BlockComponents {
    BlockComponents::new_manifests(vec![make_empty_manifest(1, 0)])
}

/// Builds terminal block components with one empty manifest at `next_l1_height`.
pub fn build_terminal_block_components(next_l1_height: L1Height) -> BlockComponents {
    BlockComponents::new_manifests(vec![make_empty_manifest(next_l1_height, 0)])
}

/// Builds terminal genesis components with transactions and one empty manifest at L1 height 1.
pub fn build_terminal_tx_components(txs: Vec<OLTransaction>) -> BlockComponents {
    BlockComponents::new(
        OLTxSegment::new(txs).expect("tx segment should be within limits"),
        Some(
            OLL1ManifestContainer::new(vec![make_empty_manifest(1, 0)])
                .expect("single manifest should succeed"),
        ),
    )
}

/// Builds and executes a chain of empty blocks starting from genesis.
///
/// Returns all completed blocks in the chain.
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

/// Builds and executes a chain of empty blocks starting from genesis.
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

    let gam_target = make_account_id(1);
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    // Create accounts before genesis
    insert_fixture_snark_account(state);
    insert_empty_account(state, gam_target);
    insert_empty_account(state, recipient_id);

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
                MsgPayload::from_bytes(BitcoinAmount::from_sat(0), msg_data.clone())
                    .expect("message payload bytes must fit within SSZ max length"),
            );
            let proof = inbox_tracker.add_message(&msg_entry);
            pending_msgs.push(msg_entry);
            pending_proofs.push(proof);

            let gam_tx = OLTransaction::new(
                OLTransactionData::from_gam_bytes(snark_acct_id, msg_data)
                    .expect("message payload bytes must fit within SSZ max length"),
                TxProofs::new_empty(),
            );
            BlockComponents::new_txs_from_ol_transactions(vec![gam_tx])
        } else if i % 4 == 3 && !pending_msgs.is_empty() {
            // Complex SnarkAccountUpdate: processes inbox messages with valid MMR proofs
            // and transfers funds to the recipient account
            let (_, account_state) = state.expect_snark_account_state(snark_acct_id);
            let builder = SnarkUpdateBuilder::from_snark_state(account_state.clone())
                .with_processed_msgs(mem::take(&mut pending_msgs))
                .with_inbox_proofs(mem::take(&mut pending_proofs))
                .with_transfer(recipient_id, 1_000_000);
            let new_state_root = make_state_root(state_root_counter);
            state_root_counter = state_root_counter.wrapping_add(1);
            let tx = builder.build(snark_acct_id, new_state_root, vec![0u8; 32]);
            BlockComponents::new_txs_from_ol_transactions(vec![tx])
        } else if i % 4 == 2 {
            // GAM to regular target account
            let gam_tx = OLTransaction::new(
                OLTransactionData::from_gam_bytes(gam_target, vec![])
                    .expect("message payload bytes must fit within SSZ max length"),
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

/// Executes a block with the given block info and returns the completed block.
pub fn execute_block(
    state: &mut impl IStateAccessorMut,
    block_info: &BlockInfo,
    parent_header: Option<&OLBlockHeader>,
    components: BlockComponents,
) -> ExecResult<CompletedBlock> {
    let block_context = BlockContext::new(block_info, parent_header);
    execute_and_complete_block(state, block_context, components, BridgeParams::default())
}

/// Executes a block and returns the construct output, which includes both the completed block and
/// execution outputs. This is useful for tests that need to inspect the logs.
pub fn execute_block_with_outputs(
    state: &mut impl IStateAccessorMut,
    block_info: &BlockInfo,
    parent_header: Option<&OLBlockHeader>,
    components: BlockComponents,
) -> ExecResult<ConstructBlockOutput> {
    let block_context = BlockContext::new(block_info, parent_header);
    construct_block(state, block_context, components, BridgeParams::default())
}

/// Executes a transaction in a non-genesis block.
///
/// Accepts an `OLTransaction` directly.
pub fn execute_tx_in_block(
    state: &mut impl IStateAccessorMut,
    parent_header: &OLBlockHeader,
    tx: OLTransaction,
    slot: Slot,
    epoch: Epoch,
) -> ExecResult<CompletedBlock> {
    let block_info = BlockInfo::new(1_001_000, slot, epoch);
    let components = BlockComponents::new_txs_from_ol_transactions(vec![tx]);
    execute_block(state, &block_info, Some(parent_header), components)
}

/// Extension helpers for read-only test state inspection.
pub trait StateTestExt: IStateAccessor {
    /// Returns snark account state, panicking if the account is missing or not a snark account.
    fn expect_snark_account_state(
        &self,
        account_id: AccountId,
    ) -> (
        &Self::AccountState,
        &<Self::AccountState as IAccountState>::SnarkAccountState,
    ) {
        let account = self
            .get_account_state(account_id)
            .unwrap_or_else(|err| panic!("account {account_id:?} lookup should succeed: {err:?}"))
            .unwrap_or_else(|| panic!("account {account_id:?} should exist"));
        (
            account,
            account.as_snark_account().unwrap_or_else(|err| {
                panic!("account {account_id:?} should be a snark account: {err:?}")
            }),
        )
    }
}

impl<S: IStateAccessor> StateTestExt for S {}

/// Asserts that a block header has the expected epoch and slot.
pub fn assert_header_position(header: &OLBlockHeader, expected_epoch: u64, expected_slot: u64) {
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

/// Asserts that state has the expected current epoch and slot.
pub fn assert_state_position(
    state: &MemoryStateBaseLayer,
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

/// Asserts that block verification succeeds.
pub fn assert_verification_succeeds<S: IStateAccessorMut>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
) {
    let result = verify_block(
        state,
        header,
        parent_header.as_ref(),
        body,
        BridgeParams::default(),
    );
    assert!(
        result.is_ok(),
        "Block verification failed when it should have succeeded: {:?}",
        result.err()
    );
}

/// Asserts that block verification fails with a specific error.
pub fn assert_verification_fails_with(
    state: &mut impl IStateAccessorMut,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
    error_matcher: impl Fn(&ExecError) -> bool,
) {
    let result = verify_block(
        state,
        header,
        parent_header.as_ref(),
        body,
        BridgeParams::default(),
    );
    assert!(
        result.is_err(),
        "Block verification succeeded when it should have failed"
    );

    let err = result.unwrap_err();
    assert!(error_matcher(&err), "Unexpected error type. Got: {:?}", err);
}

/// Returns a block header with a different parent block ID.
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

/// Returns a block header with a different state root.
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

/// Returns a block header with a different logs root.
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

/// Returns a block header with a different body root.
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

/// Returns a block header with a different slot.
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

/// Returns a block header with a different epoch.
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

/// Builder for deterministic ASM manifest fixtures.
#[derive(Debug, Clone)]
pub struct FixtureAsmManifestBuilder {
    height: L1Height,
    variant: u8,
    logs: Vec<AsmLogEntry>,
}

impl FixtureAsmManifestBuilder {
    /// Starts building a deterministic ASM manifest at `height`.
    pub fn new_at_height(height: L1Height) -> Self {
        Self {
            height,
            variant: height as u8,
            logs: Vec::new(),
        }
    }

    /// Sets the deterministic block ID and wtxids-root variant.
    pub fn with_variant(mut self, variant: u8) -> Self {
        self.variant = variant;
        self
    }

    /// Adds one ASM log.
    pub fn with_log(mut self, log: AsmLogEntry) -> Self {
        self.logs.push(log);
        self
    }

    /// Adds ASM logs.
    pub fn with_logs(mut self, logs: impl IntoIterator<Item = AsmLogEntry>) -> Self {
        self.logs.extend(logs);
        self
    }

    /// Builds the manifest.
    pub fn build(self) -> AsmManifest {
        AsmManifest::new(
            self.height,
            L1BlockId::from(Buf32::from([self.variant; 32])),
            WtxidsRoot::from(Buf32::from([self.variant; 32])),
            self.logs,
        )
        .expect("test manifest should be valid")
    }
}

/// Builder for protocol-shaped OL STF test fixtures.
#[derive(Debug)]
pub struct OLStfFixtureBuilder {
    state: MemoryStateBaseLayer,
    manifests: Vec<AsmManifest>,
}

impl OLStfFixtureBuilder {
    fn new() -> Self {
        Self {
            state: make_genesis_state(),
            manifests: vec![],
        }
    }

    /// Returns the serial that the next directly seeded account will receive.
    pub fn next_account_serial(&self) -> AccountSerial {
        self.state.next_account_serial()
    }

    /// Seeds a snark account before executing genesis.
    ///
    /// This is test setup, not the production genesis-account creation path.
    pub fn with_genesis_snark_account(
        mut self,
        account_id: AccountId,
        build: impl FnOnce(FixtureSnarkAccountBuilder) -> FixtureSnarkAccountBuilder,
    ) -> Self {
        self.insert_snark_account_with_settings(
            account_id,
            build(FixtureSnarkAccountBuilder::new()),
        );
        self
    }

    /// Seeds an empty account before executing genesis.
    pub fn with_genesis_empty_account(mut self, account_id: AccountId) -> Self {
        self.insert_empty_account(account_id);
        self
    }

    /// Sets the genesis manifest container to exactly one caller-provided manifest.
    pub fn with_genesis_manifest(mut self, manifest: AsmManifest) -> Self {
        self.manifests = vec![manifest];
        self
    }

    /// Sets the genesis manifest container to exactly these manifests.
    pub fn with_genesis_manifests(
        mut self,
        manifests: impl IntoIterator<Item = AsmManifest>,
    ) -> Self {
        self.manifests = manifests.into_iter().collect();
        self
    }

    /// Executes terminal genesis and returns the live fixture.
    pub fn execute_genesis(self) -> OLStfFixture {
        self.execute_genesis_result()
            .expect("fixture genesis should execute")
    }

    /// Executes terminal genesis and returns the live fixture plus outputs.
    pub fn execute_genesis_with_outputs(mut self) -> FixtureGenesisOutput {
        let genesis_info = BlockInfo::new_genesis(1_000_000);
        let genesis_components = BlockComponents::new_manifests(self.manifests);
        let output =
            execute_block_with_outputs(&mut self.state, &genesis_info, None, genesis_components)
                .expect("fixture genesis should execute");
        let fixture =
            OLStfFixture::from_executed_genesis(self.state, output.completed_block().clone());

        FixtureGenesisOutput { fixture, output }
    }

    fn execute_genesis_result(mut self) -> ExecResult<OLStfFixture> {
        let genesis_info = BlockInfo::new_genesis(1_000_000);
        let genesis_components = BlockComponents::new_manifests(self.manifests);
        let genesis_block =
            execute_block(&mut self.state, &genesis_info, None, genesis_components)?;

        Ok(OLStfFixture::from_executed_genesis(
            self.state,
            genesis_block,
        ))
    }

    fn insert_snark_account_with_settings(
        &mut self,
        account_id: AccountId,
        builder: FixtureSnarkAccountBuilder,
    ) -> AccountSerial {
        self.state
            .create_new_account(account_id, builder.into_new_account_data())
            .expect("should insert genesis snark account")
    }

    fn insert_empty_account(&mut self, account_id: AccountId) -> AccountSerial {
        let account = NewAccountData::new_empty(NewAccountTypeState::Empty);
        self.state
            .create_new_account(account_id, account)
            .expect("should insert genesis empty account")
    }
}

impl Default for OLStfFixtureBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Behavior-level OL STF fixture for tests.
#[derive(Debug)]
pub struct OLStfFixture {
    state: MemoryStateBaseLayer,
    last_block: CompletedBlock,
    next_slot: Slot,
    next_epoch: Epoch,
    next_timestamp: u64,
}

impl OLStfFixture {
    /// Starts configuring a protocol-shaped fixture from genesis.
    pub fn builder() -> OLStfFixtureBuilder {
        OLStfFixtureBuilder::new()
    }

    fn from_executed_genesis(state: MemoryStateBaseLayer, genesis_block: CompletedBlock) -> Self {
        Self {
            state,
            last_block: genesis_block,
            next_slot: 1,
            next_epoch: 1,
            next_timestamp: 1_001_000,
        }
    }

    /// Returns the current fixture state.
    pub fn state(&self) -> &MemoryStateBaseLayer {
        &self.state
    }

    /// Returns the mutable current fixture state.
    pub fn state_mut(&mut self) -> &mut MemoryStateBaseLayer {
        &mut self.state
    }

    /// Inserts a snark account after genesis by direct fixture state mutation.
    ///
    /// Use this for setup between blocks, not to model transaction execution.
    pub fn insert_snark_account(
        &mut self,
        account_id: AccountId,
        build: impl FnOnce(FixtureSnarkAccountBuilder) -> FixtureSnarkAccountBuilder,
    ) -> AccountSerial {
        self.insert_snark_account_with_settings(
            account_id,
            build(FixtureSnarkAccountBuilder::new()),
        )
    }

    /// Inserts an empty account after genesis by direct fixture state mutation.
    ///
    /// Use this for setup between blocks, not to model transaction execution.
    pub fn insert_empty_account(&mut self, account_id: AccountId) -> AccountSerial {
        let account = NewAccountData::new_empty(NewAccountTypeState::Empty);
        self.state
            .create_new_account(account_id, account)
            .expect("should insert fixture empty account")
    }

    /// Sets a snark account sequence number by direct fixture state mutation.
    ///
    /// Use this for boundary setup, not to model transaction execution.
    /// Panics if `account_id` is not a snark account; use only after
    /// [`OLStfFixtureBuilder::with_genesis_snark_account`] or [`Self::insert_snark_account`].
    pub fn set_account_seqno(&mut self, account_id: AccountId, seqno: u64) {
        self.state
            .update_account(account_id, |account| {
                let account_state = account
                    .as_snark_account_mut()
                    .expect("account should be a snark account");
                account_state.set_proof_state_directly(
                    account_state.inner_state_root(),
                    account_state.next_inbox_msg_idx(),
                    Seqno::from(seqno),
                );
            })
            .expect("snark account seqno should update");
    }

    /// Captures a structural snapshot of selected accounts.
    pub fn snapshot(&self, accounts: impl IntoIterator<Item = AccountId>) -> StateSnapshot {
        let accounts = accounts
            .into_iter()
            .map(|account_id| {
                let account = self
                    .state
                    .get_account_state(account_id)
                    .expect("account lookup should succeed")
                    .cloned();
                (account_id, account)
            })
            .collect();

        StateSnapshot { accounts }
    }

    /// Returns the current parent header.
    pub fn parent_header(&self) -> &OLBlockHeader {
        self.last_block.header()
    }

    /// Returns the last completed block.
    pub fn last_completed_block(&self) -> &CompletedBlock {
        &self.last_block
    }

    /// Computes the current state root.
    pub fn compute_state_root(&self) -> Buf32 {
        self.state
            .compute_state_root()
            .expect("fixture state root should compute")
    }

    /// Starts a child block builder.
    pub fn child_block(&mut self) -> FixtureBlockBuilder<'_> {
        FixtureBlockBuilder {
            fixture: self,
            txs: Vec::new(),
            manifests: Vec::new(),
            pending_seqnos: BTreeMap::new(),
            slot: None,
            epoch: None,
            timestamp: None,
        }
    }

    /// Builds an owned SAU transaction from fixture state.
    pub fn sau_tx(
        &self,
        sender: AccountId,
        build: impl FnOnce(FixtureSauBuilder) -> FixtureSauBuilder,
    ) -> OLTransaction {
        let account_state = self.expect_snark_account(sender).clone();
        let seqno = *account_state.seqno().inner();
        build(FixtureSauBuilder::new(sender, account_state, seqno))
            .build_tx_with_seqno()
            .0
    }

    /// Builds an owned GAM transaction.
    pub fn gam_tx(
        &self,
        target: AccountId,
        build: impl FnOnce(FixtureGamBuilder) -> FixtureGamBuilder,
    ) -> OLTransaction {
        build(FixtureGamBuilder::new(target)).build_tx()
    }

    /// Returns the current account state.
    pub fn expect_account(&self, account_id: AccountId) -> &OLAccountState {
        self.state
            .get_account_state(account_id)
            .expect("account lookup should succeed")
            .unwrap_or_else(|| panic!("account {account_id:?} should exist"))
    }

    /// Returns the current snark account state.
    pub fn expect_snark_account(&self, account_id: AccountId) -> &OLSnarkAccountState {
        let account = self
            .state
            .get_account_state(account_id)
            .unwrap_or_else(|err| panic!("account {account_id:?} lookup should succeed: {err:?}"))
            .unwrap_or_else(|| panic!("account {account_id:?} should exist"));
        account.as_snark_account().unwrap_or_else(|err| {
            panic!("account {account_id:?} should be a snark account: {err:?}")
        })
    }

    /// Returns the current account balance.
    pub fn account_balance(&self, account_id: AccountId) -> BitcoinAmount {
        self.expect_account(account_id).balance()
    }

    /// Returns the current account serial.
    pub fn account_serial(&self, account_id: AccountId) -> AccountSerial {
        self.expect_account(account_id).serial()
    }

    fn insert_snark_account_with_settings(
        &mut self,
        account_id: AccountId,
        builder: FixtureSnarkAccountBuilder,
    ) -> AccountSerial {
        self.state
            .create_new_account(account_id, builder.into_new_account_data())
            .expect("should insert fixture snark account")
    }
}

/// Structural snapshot of selected fixture accounts.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    accounts: BTreeMap<AccountId, Option<OLAccountState>>,
}

impl StateSnapshot {
    /// Asserts that every snapshotted account is structurally unchanged.
    ///
    /// [`OLAccountState`] equality is structural over the SSZ fields: account
    /// serial, balance, account type, and for snark accounts the update key,
    /// sequence number, proof state, and inbox MMR.
    pub fn assert_unchanged(&self, fixture: &OLStfFixture) {
        for (account_id, expected_account) in &self.accounts {
            let current_account = fixture
                .state()
                .get_account_state(*account_id)
                .expect("account lookup should succeed")
                .cloned();
            assert_eq!(
                current_account.as_ref(),
                expected_account.as_ref(),
                "account {account_id:?} should be unchanged"
            );
        }
    }
}

/// Builder for fixture-created snark accounts.
///
/// Fixture-created accounts are direct test setup. Tests that cover production
/// account creation should use the corresponding STF path instead.
#[derive(Debug, Clone)]
pub struct FixtureSnarkAccountBuilder {
    balance: BitcoinAmount,
    update_vk: PredicateKey,
    initial_state_root: Hash,
}

impl FixtureSnarkAccountBuilder {
    fn new() -> Self {
        Self {
            balance: BitcoinAmount::zero(),
            update_vk: PredicateKey::always_accept(),
            initial_state_root: make_state_root(1),
        }
    }

    /// Sets the initial balance.
    pub fn with_balance(mut self, balance: BitcoinAmount) -> Self {
        self.balance = balance;
        self
    }

    /// Sets the initial snark state root.
    pub fn with_state_root(mut self, state_root: Hash) -> Self {
        self.initial_state_root = state_root;
        self
    }

    /// Sets the update predicate key.
    pub fn with_update_vk(mut self, update_vk: PredicateKey) -> Self {
        self.update_vk = update_vk;
        self
    }

    fn into_new_account_data(self) -> NewAccountData {
        NewAccountData::new(
            self.balance,
            NewAccountTypeState::Snark {
                update_vk: self.update_vk,
                initial_state_root: self.initial_state_root,
            },
        )
    }
}

/// Builder for one fixture child block.
#[derive(Debug)]
pub struct FixtureBlockBuilder<'a> {
    fixture: &'a mut OLStfFixture,
    txs: Vec<OLTransaction>,
    manifests: Vec<AsmManifest>,
    pending_seqnos: BTreeMap<AccountId, u64>,
    slot: Option<Slot>,
    epoch: Option<Epoch>,
    timestamp: Option<u64>,
}

impl<'a> FixtureBlockBuilder<'a> {
    /// Sets the child block slot.
    pub fn with_slot(mut self, slot: Slot) -> Self {
        self.slot = Some(slot);
        self
    }

    /// Sets the child block epoch.
    pub fn with_epoch(mut self, epoch: Epoch) -> Self {
        self.epoch = Some(epoch);
        self
    }

    /// Adds a caller-built transaction.
    pub fn with_tx(mut self, tx: OLTransaction) -> Self {
        self.txs.push(tx);
        self
    }

    /// Adds an ASM manifest to this terminal block.
    pub fn with_manifest(mut self, manifest: AsmManifest) -> Self {
        self.manifests.push(manifest);
        self
    }

    /// Adds ASM manifests to this terminal block.
    pub fn with_manifests(mut self, manifests: impl IntoIterator<Item = AsmManifest>) -> Self {
        self.manifests.extend(manifests);
        self
    }

    /// Adds a SAU transaction built from fixture state.
    pub fn with_sau(
        mut self,
        sender: AccountId,
        build: impl FnOnce(FixtureSauBuilder) -> FixtureSauBuilder,
    ) -> Self {
        let account = self
            .fixture
            .state
            .get_account_state(sender)
            .expect("account lookup should succeed");
        let account_state = account
            .as_ref()
            .and_then(|account| account.as_snark_account().ok().cloned());
        let persisted_seqno = account_state
            .as_ref()
            .map(|account_state| *account_state.seqno().inner())
            .unwrap_or(0);
        let seqno = *self.pending_seqnos.entry(sender).or_insert(persisted_seqno);
        let builder = match account_state {
            Some(account_state) => FixtureSauBuilder::new(sender, account_state, seqno),
            None => FixtureSauBuilder::new_unchecked(sender, seqno),
        };
        let (tx, maybe_used_seqno) = build(builder).build_tx_with_seqno();
        if let Some(used_seqno) = maybe_used_seqno {
            self.pending_seqnos.insert(sender, used_seqno + 1);
        }
        self.txs.push(tx);
        self
    }

    /// Adds a default empty-payload GAM transaction.
    pub fn with_default_gam(self, target: AccountId) -> Self {
        self.with_gam(target, |gam| gam)
    }

    /// Adds a GAM transaction.
    pub fn with_gam(
        mut self,
        target: AccountId,
        build: impl FnOnce(FixtureGamBuilder) -> FixtureGamBuilder,
    ) -> Self {
        self.txs
            .push(build(FixtureGamBuilder::new(target)).build_tx());
        self
    }

    /// Executes the block and returns its outcome.
    pub fn execute(self) -> FixtureBlockOutcome {
        self.execute_result()
            .expect("fixture child block should execute")
    }

    /// Executes the block and returns its completed block plus execution outputs.
    pub fn execute_with_outputs(self) -> FixtureBlockOutput {
        self.execute_with_outputs_result()
            .expect("fixture child block should execute")
    }

    /// Executes the block and expects an error.
    pub fn execute_err(self) -> ExecError {
        self.execute_result()
            .expect_err("fixture child block should fail")
    }

    fn execute_result(self) -> ExecResult<FixtureBlockOutcome> {
        let Self {
            fixture,
            txs,
            manifests,
            pending_seqnos: _,
            slot,
            epoch,
            timestamp,
        } = self;
        let slot = slot.unwrap_or(fixture.next_slot);
        let epoch = epoch.unwrap_or(fixture.next_epoch);
        let timestamp = timestamp.unwrap_or(fixture.next_timestamp);
        let block_info = BlockInfo::new(timestamp, slot, epoch);
        let is_terminal = !manifests.is_empty();
        let components = Self::components_from(txs, manifests);
        let parent_header = fixture.last_block.header().clone();
        let block = execute_block(
            &mut fixture.state,
            &block_info,
            Some(&parent_header),
            components,
        )?;

        fixture.next_slot = slot + 1;
        fixture.next_epoch = epoch + u32::from(is_terminal);
        fixture.next_timestamp = timestamp + 1_000;
        fixture.last_block = block.clone();

        Ok(FixtureBlockOutcome {
            completed_block: block,
        })
    }

    fn execute_with_outputs_result(self) -> ExecResult<FixtureBlockOutput> {
        let Self {
            fixture,
            txs,
            manifests,
            pending_seqnos: _,
            slot,
            epoch,
            timestamp,
        } = self;
        let slot = slot.unwrap_or(fixture.next_slot);
        let epoch = epoch.unwrap_or(fixture.next_epoch);
        let timestamp = timestamp.unwrap_or(fixture.next_timestamp);
        let block_info = BlockInfo::new(timestamp, slot, epoch);
        let is_terminal = !manifests.is_empty();
        let components = Self::components_from(txs, manifests);
        let parent_header = fixture.last_block.header().clone();
        let output = execute_block_with_outputs(
            &mut fixture.state,
            &block_info,
            Some(&parent_header),
            components,
        )?;

        fixture.next_slot = slot + 1;
        fixture.next_epoch = epoch + u32::from(is_terminal);
        fixture.next_timestamp = timestamp + 1_000;
        fixture.last_block = output.completed_block().clone();

        Ok(FixtureBlockOutput { output })
    }

    fn components_from(txs: Vec<OLTransaction>, manifests: Vec<AsmManifest>) -> BlockComponents {
        if manifests.is_empty() {
            return BlockComponents::new_txs_from_ol_transactions(txs);
        }

        BlockComponents::new(
            OLTxSegment::new(txs).expect("tx segment should be within limits"),
            Some(OLL1ManifestContainer::new(manifests).expect("manifests should be within limits")),
        )
    }
}

/// Fixture SAU builder.
///
/// Bare methods such as [`FixtureSauBuilder::transfer`] and
/// [`FixtureSauBuilder::output_message`] add operation effects. `with_*`
/// methods override envelope fields. `force_*` methods intentionally build
/// invalid updates for negative-path tests and do not advance same-block
/// fixture sequence tracking.
///
/// The fixture tracks same-block sequence numbers only. It does not track
/// same-block staged `next_inbox_msg_idx` changes between SAUs.
#[derive(Debug)]
pub struct FixtureSauBuilder {
    sender: AccountId,
    builder: SnarkUpdateBuilder,
    state_root: Hash,
    proof: Vec<u8>,
    should_advance_seqno: bool,
}

impl FixtureSauBuilder {
    fn new(sender: AccountId, account_state: OLSnarkAccountState, seqno: u64) -> Self {
        let builder = SnarkUpdateBuilder::from_snark_state(account_state).with_seq_no(seqno);
        Self {
            sender,
            builder,
            state_root: make_state_root((seqno as u8).wrapping_add(2)),
            proof: make_proof(1),
            should_advance_seqno: true,
        }
    }

    fn new_unchecked(sender: AccountId, seqno: u64) -> Self {
        Self {
            sender,
            builder: SnarkUpdateBuilder::new_unchecked(seqno, 0),
            state_root: make_state_root((seqno as u8).wrapping_add(2)),
            proof: make_proof(1),
            should_advance_seqno: true,
        }
    }

    /// Adds a transfer effect.
    pub fn transfer(mut self, dest: AccountId, amount: BitcoinAmount) -> Self {
        self.builder = self.builder.with_transfer(dest, amount.to_sat());
        self
    }

    /// Adds an output message effect.
    pub fn output_message(
        mut self,
        dest: AccountId,
        value: BitcoinAmount,
        payload: Vec<u8>,
    ) -> Self {
        self.builder = self
            .builder
            .with_output_message(dest, value.to_sat(), payload);
        self
    }

    /// Adds ledger reference claims and proofs.
    pub fn with_ledger_refs(
        mut self,
        claims: Vec<AccumulatorClaim>,
        proofs: Vec<RawMerkleProof>,
    ) -> Self {
        self.builder = self.builder.with_ledger_refs(claims, proofs);
        self
    }

    /// Adds processed inbox messages and their proofs.
    ///
    /// If [`Self::force_next_inbox_msg_idx`] is also used, the forced index
    /// overrides the index derived from these messages.
    pub fn with_processed_messages(
        mut self,
        messages: Vec<MessageEntry>,
        proofs: Vec<RawMerkleProof>,
    ) -> Self {
        self.builder = self
            .builder
            .with_processed_msgs(messages)
            .with_inbox_proofs(proofs);
        self
    }

    /// Adds extra data to the SAU payload.
    pub fn with_extra_data(mut self, extra_data: Vec<u8>) -> Self {
        self.builder = self
            .builder
            .try_with_extra_data(extra_data)
            .expect("fixture SAU extra data should fit within SSZ bound");
        self
    }

    /// Forces an explicit sequence number for negative-path tests.
    pub fn force_seqno(mut self, seq_no: u64) -> Self {
        self.builder = self.builder.with_seq_no(seq_no);
        self.should_advance_seqno = false;
        self
    }

    /// Forces the resulting inbox message index for negative-path tests.
    pub fn force_next_inbox_msg_idx(mut self, next_msg_idx: u64) -> Self {
        self.builder = self.builder.with_new_msg_idx(next_msg_idx);
        self.should_advance_seqno = false;
        self
    }

    /// Overrides the resulting snark state root.
    pub fn with_state_root(mut self, state_root: Hash) -> Self {
        self.state_root = state_root;
        self
    }

    /// Overrides the predicate proof bytes.
    pub fn with_proof(mut self, proof: Vec<u8>) -> Self {
        self.proof = proof;
        self
    }

    fn build_tx_with_seqno(self) -> (OLTransaction, Option<u64>) {
        let seqno = self.builder.seq_no();
        (
            self.builder.build(self.sender, self.state_root, self.proof),
            self.should_advance_seqno.then_some(seqno),
        )
    }
}

/// Fixture GAM builder.
#[derive(Debug)]
pub struct FixtureGamBuilder {
    target: AccountId,
    payload: Vec<u8>,
}

impl FixtureGamBuilder {
    fn new(target: AccountId) -> Self {
        Self {
            target,
            payload: Vec::new(),
        }
    }

    /// Sets the GAM payload.
    pub fn with_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = payload;
        self
    }

    fn build_tx(self) -> OLTransaction {
        OLTransaction::new(
            OLTransactionData::from_gam_bytes(self.target, self.payload)
                .expect("fixture GAM payload should fit within SSZ max length"),
            TxProofs::new_empty(),
        )
    }
}

/// Outcome from executing a fixture block.
#[derive(Debug)]
pub struct FixtureBlockOutcome {
    completed_block: CompletedBlock,
}

impl FixtureBlockOutcome {
    /// Returns the completed block.
    pub fn completed_block(&self) -> &CompletedBlock {
        &self.completed_block
    }

    /// Returns the block state root.
    pub fn state_root(&self) -> &Buf32 {
        self.completed_block.header().state_root()
    }
}

/// Outcome from executing a fixture block with execution outputs.
#[derive(Debug)]
pub struct FixtureBlockOutput {
    output: ConstructBlockOutput,
}

/// Outcome from executing fixture genesis with execution outputs.
#[derive(Debug)]
pub struct FixtureGenesisOutput {
    fixture: OLStfFixture,
    output: ConstructBlockOutput,
}

impl FixtureGenesisOutput {
    /// Returns the live fixture after genesis execution.
    pub fn fixture(&self) -> &OLStfFixture {
        &self.fixture
    }

    /// Consumes this output and returns the live fixture.
    pub fn into_fixture(self) -> OLStfFixture {
        self.fixture
    }

    /// Returns the number of logs emitted by genesis.
    pub fn log_count(&self) -> usize {
        self.output.outputs().logs().len()
    }

    /// Finds and decodes a typed log emitted by `serial`.
    pub fn find_typed_log<T: Codec>(&self, serial: AccountSerial) -> Option<T> {
        self.output
            .outputs()
            .logs()
            .iter()
            .find(|l| l.account_serial() == serial)
            .and_then(|l| decode_buf_exact::<T>(l.payload()).ok())
    }

    /// Decodes the typed log emitted by `serial`, panicking if it is missing.
    pub fn expect_typed_log<T: Codec>(&self, serial: AccountSerial) -> T {
        self.find_typed_log(serial).unwrap_or_else(|| {
            panic!(
                "expected log of type {} for account serial {serial:?}",
                type_name::<T>()
            )
        })
    }
}

impl FixtureBlockOutput {
    /// Returns the completed block.
    pub fn completed_block(&self) -> &CompletedBlock {
        self.output.completed_block()
    }

    /// Returns the block state root.
    pub fn state_root(&self) -> &Buf32 {
        self.output.completed_block().header().state_root()
    }

    /// Returns the number of logs emitted by the block.
    pub fn log_count(&self) -> usize {
        self.output.outputs().logs().len()
    }

    /// Finds and decodes a typed log emitted by `serial`.
    pub fn find_typed_log<T: Codec>(&self, serial: AccountSerial) -> Option<T> {
        self.output
            .outputs()
            .logs()
            .iter()
            .find(|l| l.account_serial() == serial)
            .and_then(|l| decode_buf_exact::<T>(l.payload()).ok())
    }

    /// Returns true if any emitted log uses `serial`.
    pub fn has_log_from_account_serial(&self, serial: AccountSerial) -> bool {
        self.output
            .outputs()
            .logs()
            .iter()
            .any(|log| log.account_serial() == serial)
    }

    /// Finds and decodes a typed log emitted by `serial`, failing if missing.
    pub fn expect_typed_log<T: Codec>(&self, serial: AccountSerial) -> T {
        self.find_typed_log(serial).unwrap_or_else(|| {
            panic!(
                "fixture block output should contain typed log {} from account serial {:?}",
                type_name::<T>(),
                serial
            )
        })
    }
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

    /// Returns the current raw proof for a tracked inbox entry.
    ///
    /// Panics if the index is not tracked.
    pub fn expect_raw_proof_at(&self, index: usize) -> RawMerkleProof {
        let proof = self.proofs.get(index).expect("test proof should exist");
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
    /// Creates a tracker matching the test genesis state (genesis L1 height 0).
    ///
    /// This prefills the tracked MMR with a single sentinel leaf so its index
    /// space lines up with `make_genesis_state`'s prefilled state MMR.
    pub fn new() -> Self {
        Self::with_genesis_l1_height(0)
    }

    /// Creates a tracker prefilled with sentinel leaves up to and including
    /// the given L1 height, so MMR indices match L1 heights.
    pub fn with_genesis_l1_height(genesis_l1_height: u32) -> Self {
        let prefill_count = genesis_l1_height as u64 + 1;
        let mmr = <Mmr64 as strata_merkle::Mmr<StrataHasher>>::new_repeated(
            MMR_SENTINEL_DUMMY_LEAF,
            prefill_count,
        );
        Self {
            mmr,
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

/// Inserts a snark account with an initial balance and executes genesis.
///
/// Genesis carries an empty manifest container, matching production genesis.
/// Use this for raw tests that need explicit STF entrypoint control. Behavior
/// tests should prefer [`OLStfFixture::builder`].
/// Returns the completed genesis block.
pub fn setup_genesis_with_snark_account(
    state: &mut impl IStateAccessorMut,
    snark_acct_id: AccountId,
    initial_balance: u64,
) -> CompletedBlock {
    setup_genesis_with_snark_accounts(state, &[(snark_acct_id, initial_balance)])
}

/// Inserts snark accounts with initial balances and executes genesis.
///
/// Genesis carries an empty manifest container, matching production genesis.
/// Use this for raw tests that need explicit STF entrypoint control. Behavior
/// tests should prefer [`OLStfFixture::builder`].
/// Returns the completed genesis block.
pub fn setup_genesis_with_snark_accounts(
    state: &mut impl IStateAccessorMut,
    accounts: &[(AccountId, u64)],
) -> CompletedBlock {
    for &(account_id, initial_balance) in accounts {
        insert_snark_account_with_settings(
            state,
            account_id,
            FixtureSnarkAccountBuilder::new()
                .with_balance(BitcoinAmount::from_sat(initial_balance)),
        );
    }

    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_components = BlockComponents::new_manifests(vec![]);
    execute_block(state, &genesis_info, None, genesis_components).expect("Genesis should execute")
}

/// Inserts the default fixture snark account in the given state.
fn insert_fixture_snark_account(state: &mut impl IStateAccessorMut) {
    insert_snark_account_with_settings(
        state,
        make_account_id(TEST_SNARK_ACCOUNT_ID),
        FixtureSnarkAccountBuilder::new().with_balance(BitcoinAmount::from_sat(100_000_000)),
    );
}

/// Inserts a snark account from fixture account settings for raw STF-entrypoint tests.
fn insert_snark_account_with_settings(
    state: &mut impl IStateAccessorMut,
    account_id: AccountId,
    builder: FixtureSnarkAccountBuilder,
) -> AccountSerial {
    state
        .create_new_account(account_id, builder.into_new_account_data())
        .expect("should insert snark account")
}

/// Inserts an empty account for raw STF-entrypoint tests.
///
/// Behavior tests should prefer [`OLStfFixtureBuilder::with_genesis_empty_account`]
/// before genesis or [`OLStfFixture::insert_empty_account`] after genesis.
pub fn insert_empty_account(
    state: &mut impl IStateAccessorMut,
    account_id: AccountId,
) -> AccountSerial {
    let new_acct_data = NewAccountData::new_empty(NewAccountTypeState::Empty);
    state
        .create_new_account(account_id, new_acct_data)
        .expect("should insert empty account")
}

/// Builder for `SnarkAccountUpdate` transactions.
///
/// The builder captures the starting snark state and derives sequence numbers
/// and message indices for the resulting update.
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
    extra_data: VariableList<u8, 1024>,
    next_msg_idx_override: Option<u64>,
}

impl SnarkUpdateBuilder {
    /// Constructs a builder from the current snark account state.
    pub fn from_snark_state(account_state: OLSnarkAccountState) -> Self {
        Self {
            seq_no: *account_state.seqno().inner(),
            old_msg_idx: account_state.next_inbox_msg_idx(),
            processed_messages: vec![],
            inbox_proofs: vec![],
            effects: TxEffects::default(),
            ledger_ref_claims: vec![],
            ledger_ref_proofs: vec![],
            extra_data: VariableList::default(),
            next_msg_idx_override: None,
        }
    }

    /// Constructs an unchecked builder when no snark account state is available.
    fn new_unchecked(seq_no: u64, old_msg_idx: u64) -> Self {
        Self {
            seq_no,
            old_msg_idx,
            processed_messages: vec![],
            inbox_proofs: vec![],
            effects: TxEffects::default(),
            ledger_ref_claims: vec![],
            ledger_ref_proofs: vec![],
            extra_data: VariableList::default(),
            next_msg_idx_override: None,
        }
    }

    fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn with_extra_data(mut self, extra_data: VariableList<u8, 1024>) -> Self {
        self.extra_data = extra_data;
        self
    }

    /// Overrides the sequence number for negative-path tests.
    pub fn with_seq_no(mut self, seq_no: u64) -> Self {
        self.seq_no = seq_no;
        self
    }

    /// Overrides the resulting inbox message index for negative-path tests.
    pub fn with_new_msg_idx(mut self, new_msg_idx: u64) -> Self {
        self.next_msg_idx_override = Some(new_msg_idx);
        self
    }

    pub fn try_with_extra_data<T: TryInto<VariableList<u8, 1024>>>(
        mut self,
        extra_data: T,
    ) -> Result<Self, T::Error> {
        self.extra_data = extra_data.try_into()?;
        Ok(self)
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
        let added = self
            .effects
            .add_transfer(SentTransfer::new(dest, BitcoinAmount::from_sat(amount)));
        // This builder only constructs test fixtures; fail fast instead of silently dropping
        // an effect that exceeds the protocol list capacity.
        assert!(added, "test: too many transfer effects");
        self
    }

    /// Add a single message effect
    pub fn with_output_message(mut self, dest: AccountId, amount: u64, data: Vec<u8>) -> Self {
        let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(amount), data)
            .expect("message payload bytes must fit within SSZ max length");
        let added = self.effects.add_message(SentMessage::new(dest, payload));
        // This builder only constructs test fixtures; fail fast instead of silently dropping
        // an effect that exceeds the protocol list capacity.
        assert!(added, "test: too many message effects");
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
        let new_msg_idx = self
            .next_msg_idx_override
            .unwrap_or(self.old_msg_idx + self.processed_messages.len() as u64);

        // Build SauTxPayload
        let proof_state = SauTxProofState {
            new_next_msg_idx: new_msg_idx,
            inner_state_root: <[u8; 32]>::from(new_state_root).into(),
        };
        let update_data = SauTxUpdateData {
            seq_no: self.seq_no,
            proof_state,
            extra_data: self.extra_data,
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
            messages: self
                .processed_messages
                .try_into()
                .expect("test: too many processed messages"),
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
