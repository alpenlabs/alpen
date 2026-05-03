use std::{collections::HashMap, slice, sync::Arc};

use async_trait::async_trait;
use proptest::prelude::*;
use ssz::Encode;
use strata_acct_types::{MessageEntry, MsgPayload};
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::{
    DbError, DbResult,
    ol_state_index::{AccountUpdateMeta, AccountUpdateRecord, InboxMessageRecord},
};
use strata_identifiers::*;
use strata_ledger_types::*;
use strata_ol_chain_types_new::*;
use strata_ol_mempool::{OLMempoolError, OLMempoolResult};
use strata_ol_params::OLParams;
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer};
use strata_ol_rpc_types::*;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::OLState;
use strata_predicate::PredicateKey;
use strata_primitives::{
    HexBytes, HexBytes32, OLBlockCommitment, epoch::EpochCommitment, prelude::BitcoinAmount,
};
use strata_snark_acct_types::{Seqno, UpdateInputData};
use strata_status::OLSyncStatus;
use tokio::runtime::Builder;

use super::OLRpcServer;
use crate::rpc::errors::{
    INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, MEMPOOL_CAPACITY_ERROR_CODE, map_mempool_error_to_rpc,
};

// -- Mock provider --

type SubmitFn = Box<dyn Fn(OLTransaction) -> OLMempoolResult<OLTxId> + Send + Sync>;
type InboxFetchFn = Box<dyn Fn(AccountId, u64, u64) -> DbResult<Vec<MessageEntry>> + Send + Sync>;

struct MockProvider {
    blocks: HashMap<OLBlockId, OLBlock>,
    canonical_slots: HashMap<Slot, OLBlockCommitment>,
    states: HashMap<OLBlockCommitment, Arc<OLState>>,
    epoch_commitments: HashMap<Epoch, EpochCommitment>,
    epoch_summaries: HashMap<EpochCommitment, EpochSummary>,
    checkpoint_l1_refs: HashMap<EpochCommitment, CheckpointL1Ref>,
    account_update_entries: HashMap<(Epoch, AccountId), Vec<AccountUpdateRecord>>,
    account_inbox_entries: HashMap<(Epoch, AccountId), Vec<InboxMessageRecord>>,
    account_creation_epochs: HashMap<AccountId, Epoch>,
    manifests: HashMap<L1Height, AsmManifest>,
    l1_tip_height: Option<L1Height>,
    sync_status: Option<OLSyncStatus>,
    submit_fn: SubmitFn,
    inbox_fetch_fn: Option<InboxFetchFn>,
}

impl MockProvider {
    fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            canonical_slots: HashMap::new(),
            states: HashMap::new(),
            epoch_commitments: HashMap::new(),
            epoch_summaries: HashMap::new(),
            checkpoint_l1_refs: HashMap::new(),
            account_update_entries: HashMap::new(),
            account_inbox_entries: HashMap::new(),
            account_creation_epochs: HashMap::new(),
            manifests: HashMap::new(),
            l1_tip_height: None,
            sync_status: None,
            submit_fn: Box::new(|_| Ok(OLTxId::from(Buf32::from([0xAB; 32])))),
            inbox_fetch_fn: None,
        }
    }

    fn with_sync_status(mut self, status: OLSyncStatus) -> Self {
        self.sync_status = Some(status);
        self
    }

    fn with_block_and_state(mut self, block: &OLBlock, state: OLState) -> Self {
        let blkid = block.header().compute_blkid();
        let slot = block.header().slot();
        let commitment = OLBlockCommitment::new(slot, blkid);
        self.blocks.insert(blkid, block.clone());
        self.canonical_slots.insert(slot, commitment);
        self.states.insert(commitment, Arc::new(state));
        self
    }

    fn with_epoch_commitment(mut self, epoch: Epoch, commitment: EpochCommitment) -> Self {
        self.epoch_commitments.insert(epoch, commitment);
        self
    }

    fn with_epoch_summary(mut self, summary: EpochSummary) -> Self {
        self.epoch_summaries
            .insert(summary.get_epoch_commitment(), summary);
        self
    }

    fn with_checkpoint_l1_ref(
        mut self,
        commitment: EpochCommitment,
        l1_ref: CheckpointL1Ref,
    ) -> Self {
        self.checkpoint_l1_refs.insert(commitment, l1_ref);
        self
    }

    fn with_l1_tip_height(mut self, height: L1Height) -> Self {
        self.l1_tip_height = Some(height);
        self
    }

    fn with_manifest(mut self, manifest: AsmManifest) -> Self {
        self.manifests.insert(manifest.height(), manifest);
        self
    }

    fn with_state_at(mut self, commitment: OLBlockCommitment, state: OLState) -> Self {
        self.states.insert(commitment, Arc::new(state));
        self
    }

    fn with_snark_state_at_terminal(
        self,
        commitment: EpochCommitment,
        account_id: AccountId,
        seq_no: u64,
        next_inbox_msg_idx: u64,
    ) -> Self {
        self.with_state_at(
            commitment.to_block_commitment(),
            ol_state_with_snark_account(
                account_id,
                commitment.last_slot(),
                seq_no,
                next_inbox_msg_idx,
            ),
        )
    }

    fn with_genesis_state_at_terminal(self, commitment: EpochCommitment) -> Self {
        self.with_state_at(commitment.to_block_commitment(), genesis_ol_state())
    }

    fn with_account_extra_data(
        mut self,
        account_id: AccountId,
        epoch: Epoch,
        seq_no: u64,
        next_inbox_idx: u64,
        extra_data: Vec<u8>,
        block: OLBlockCommitment,
    ) -> Self {
        let meta = AccountUpdateMeta::new(block, [0u8; 32].into());
        let record = AccountUpdateRecord::new(Some(meta), seq_no, next_inbox_idx, Some(extra_data));
        self.account_update_entries
            .entry((epoch, account_id))
            .or_default()
            .push(record);
        self
    }

    fn with_account_extra_data_at_terminal(
        self,
        account_id: AccountId,
        epoch: Epoch,
        seq_no: u64,
        next_inbox_idx: u64,
        extra_data: Vec<u8>,
        commitment: EpochCommitment,
    ) -> Self {
        self.with_account_extra_data(
            account_id,
            epoch,
            seq_no,
            next_inbox_idx,
            extra_data,
            commitment.to_block_commitment(),
        )
    }

    fn with_account_update_records(
        mut self,
        account_id: AccountId,
        epoch: Epoch,
        records: Vec<AccountUpdateRecord>,
    ) -> Self {
        self.account_update_entries
            .insert((epoch, account_id), records);
        self
    }

    fn with_account_inbox_records(
        mut self,
        account_id: AccountId,
        epoch: Epoch,
        records: Vec<InboxMessageRecord>,
    ) -> Self {
        self.account_inbox_entries
            .insert((epoch, account_id), records);
        self
    }

    fn with_submit_fn(
        mut self,
        f: impl Fn(OLTransaction) -> OLMempoolResult<OLTxId> + Send + Sync + 'static,
    ) -> Self {
        self.submit_fn = Box::new(f);
        self
    }

    fn with_inbox_fetch_fn(
        mut self,
        f: impl Fn(AccountId, u64, u64) -> DbResult<Vec<MessageEntry>> + Send + Sync + 'static,
    ) -> Self {
        self.inbox_fetch_fn = Some(Box::new(f));
        self
    }
}

#[async_trait]
impl OLRpcProvider for MockProvider {
    async fn get_canonical_block_at(&self, height: u64) -> DbResult<Option<OLBlockCommitment>> {
        Ok(self.canonical_slots.get(&height).copied())
    }

    async fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlock>> {
        Ok(self.blocks.get(&id).cloned())
    }

    async fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLState>>> {
        Ok(self.states.get(&commitment).cloned())
    }

    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        Ok(self.epoch_commitments.get(&epoch).copied())
    }

    async fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<EpochSummary>> {
        Ok(self.epoch_summaries.get(&commitment).copied())
    }

    async fn get_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>> {
        Ok(self.checkpoint_l1_refs.get(&commitment).cloned())
    }

    async fn get_account_update_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<AccountUpdateRecord>>> {
        Ok(self.account_update_entries.get(&(epoch, account)).cloned())
    }

    async fn get_account_inbox_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<InboxMessageRecord>>> {
        Ok(self.account_inbox_entries.get(&(epoch, account)).cloned())
    }

    async fn get_account_inbox_messages(
        &self,
        account_id: AccountId,
        start_idx: u64,
        end_idx_exclusive: u64,
    ) -> DbResult<Vec<MessageEntry>> {
        if let Some(fetch_fn) = &self.inbox_fetch_fn {
            return fetch_fn(account_id, start_idx, end_idx_exclusive);
        }

        if end_idx_exclusive <= start_idx {
            return Ok(Vec::new());
        }

        Ok(Vec::new())
    }

    async fn get_account_creation_epoch(&self, account_id: AccountId) -> DbResult<Option<Epoch>> {
        Ok(self.account_creation_epochs.get(&account_id).copied())
    }

    async fn get_block_manifest_at_height(
        &self,
        height: L1Height,
    ) -> DbResult<Option<AsmManifest>> {
        Ok(self.manifests.get(&height).cloned())
    }

    fn get_ol_sync_status(&self) -> Option<OLSyncStatus> {
        self.sync_status
    }

    fn get_l1_tip_height(&self) -> Option<L1Height> {
        self.l1_tip_height
    }

    async fn submit_transaction(&self, tx: OLTransaction) -> OLMempoolResult<OLTxId> {
        (self.submit_fn)(tx)
    }
}

// -- Helpers --

fn test_account_id(byte: u8) -> AccountId {
    let mut bytes = [1u8; 32];
    bytes[0] = byte;
    AccountId::new(bytes)
}

fn fixed_buf32(tag: u8) -> Buf32 {
    let mut bytes = [0u8; 32];
    bytes[0] = tag;
    Buf32::from(bytes)
}

fn fixed_l1_block_id(tag: u8) -> L1BlockId {
    L1BlockId::from(fixed_buf32(tag))
}

fn fixed_ol_block_id(tag: u8) -> OLBlockId {
    OLBlockId::from(fixed_buf32(tag))
}

fn test_l1_commitment() -> L1BlockCommitment {
    L1BlockCommitment::new(0, L1BlockId::default())
}

fn null_blkid() -> OLBlockId {
    OLBlockId::from(Buf32::zero())
}

fn make_sync_status(
    tip: OLBlockCommitment,
    tip_epoch: Epoch,
    tip_is_terminal: bool,
    prev_epoch: EpochCommitment,
    confirmed_epoch: EpochCommitment,
    finalized_epoch: EpochCommitment,
) -> OLSyncStatus {
    OLSyncStatus::new(
        tip,
        tip_epoch,
        tip_is_terminal,
        prev_epoch,
        confirmed_epoch,
        finalized_epoch,
        test_l1_commitment(),
    )
}

fn make_block(slot: Slot, epoch: Epoch, parent: OLBlockId) -> OLBlock {
    let header = OLBlockHeader::new(
        0,
        0.into(),
        slot,
        epoch,
        parent,
        Buf32::zero(),
        Buf32::zero(),
        Buf32::zero(),
    );
    let signed = SignedOLBlockHeader::new(header, Buf64::zero());
    let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty segment"));
    OLBlock::new(signed, body)
}

fn genesis_ol_state() -> OLState {
    let params = OLParams::new_empty(test_l1_commitment());
    OLState::from_genesis_params(&params).expect("genesis state")
}

fn ol_state_with_snark_account(
    account_id: AccountId,
    slot: Slot,
    seq_no: u64,
    next_inbox_msg_idx: u64,
) -> OLState {
    ol_state_with_snark_account_and_inbox_entries(account_id, slot, seq_no, next_inbox_msg_idx, &[])
}

fn ol_state_with_snark_account_and_inbox_entries(
    account_id: AccountId,
    slot: Slot,
    seq_no: u64,
    next_inbox_msg_idx: u64,
    inbox_messages: &[MessageEntry],
) -> OLState {
    let base = genesis_ol_state();
    let mut state = MemoryStateBaseLayer::new(base);
    state.set_cur_slot(slot);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from(0),
        NewAccountTypeState::Snark {
            update_vk: PredicateKey::always_accept(),
            initial_state_root: Hash::zero(),
        },
    );
    state.create_new_account(account_id, new_acct).unwrap();
    state
        .update_account(account_id, |acct| {
            let s = acct.as_snark_account_mut().unwrap();
            s.set_proof_state_directly(Hash::zero(), next_inbox_msg_idx, Seqno::from(seq_no));
            for message in inbox_messages {
                s.insert_inbox_message(message.clone()).unwrap();
            }
        })
        .unwrap();
    state.into_inner()
}

fn ol_state_with_empty_account(account_id: AccountId, slot: Slot) -> OLState {
    let base = genesis_ol_state();
    let mut state = MemoryStateBaseLayer::new(base);
    state.set_cur_slot(slot);
    let new_acct = NewAccountData::new(BitcoinAmount::from(0), NewAccountTypeState::Empty);
    state.create_new_account(account_id, new_acct).unwrap();
    state.into_inner()
}

const TEST_GENESIS_L1_HEIGHT: L1Height = 0;

const TEST_MAX_HEADERS_RANGE: usize = 5000;
const DEFAULT_NEXT_INBOX_MSG_IDX: u64 = 0;

fn make_rpc(provider: MockProvider) -> OLRpcServer<MockProvider> {
    OLRpcServer::new(provider, TEST_GENESIS_L1_HEIGHT, TEST_MAX_HEADERS_RANGE)
}

fn make_gam_rpc_tx(target: AccountId, payload: Vec<u8>) -> RpcOLTransaction {
    let gam = RpcGenericAccountMessage::new(HexBytes32::from(*target.inner()), HexBytes(payload));
    RpcOLTransaction::new_payload(RpcTransactionPayload::GenericAccountMessage(gam))
}

fn test_epoch_commitment(epoch: Epoch, slot: Slot, blkid_tag: u8) -> EpochCommitment {
    EpochCommitment::new(epoch, slot, fixed_ol_block_id(blkid_tag))
}

fn make_message_entry(
    source: AccountId,
    incl_epoch: Epoch,
    payload_value_sat: u64,
    payload_buf: Vec<u8>,
) -> MessageEntry {
    let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(payload_value_sat), payload_buf)
        .expect("message payload bytes must fit within SSZ max length");
    MessageEntry::new(source, incl_epoch, payload)
}

fn rpc_messages_to_entries(messages: &[RpcMessageEntry]) -> Vec<MessageEntry> {
    messages
        .iter()
        .cloned()
        .map(|msg| {
            msg.try_into()
                .expect("message payload bytes must fit within SSZ max length")
        })
        .collect()
}

fn rpc_update_to_input(update: RpcUpdateInputData) -> UpdateInputData {
    update
        .try_into()
        .expect("message payload bytes must fit within SSZ max length")
}

fn inbox_fetch_expect_success(
    expected_account_id: AccountId,
    expected_start_idx: u64,
    expected_end_idx_exclusive: u64,
    messages_to_return: Vec<MessageEntry>,
) -> impl Fn(AccountId, u64, u64) -> DbResult<Vec<MessageEntry>> + Send + Sync + 'static {
    move |queried_account_id, start_idx, end_idx_exclusive| {
        assert_eq!(queried_account_id, expected_account_id);
        assert_eq!(start_idx, expected_start_idx);
        assert_eq!(end_idx_exclusive, expected_end_idx_exclusive);
        Ok(messages_to_return.clone())
    }
}

/// Returns whichever messages from `indexed_messages` have global indices
/// in the queried `[start_idx, end_idx_exclusive)` range. Decoupled from
/// call order or batching — exercises behavior, not call protocol.
fn inbox_fetch_in_range(
    expected_account_id: AccountId,
    indexed_messages: Vec<(u64, MessageEntry)>,
) -> impl Fn(AccountId, u64, u64) -> DbResult<Vec<MessageEntry>> + Send + Sync + 'static {
    move |queried_account_id, start_idx, end_idx_exclusive| {
        assert_eq!(queried_account_id, expected_account_id);
        let mut msgs: Vec<(u64, MessageEntry)> = indexed_messages
            .iter()
            .filter(|(idx, _)| *idx >= start_idx && *idx < end_idx_exclusive)
            .cloned()
            .collect();
        msgs.sort_by_key(|(idx, _)| *idx);
        Ok(msgs.into_iter().map(|(_, m)| m).collect())
    }
}

fn inbox_fetch_panic(
    message: &'static str,
) -> impl Fn(AccountId, u64, u64) -> DbResult<Vec<MessageEntry>> + Send + Sync + 'static {
    move |_, _, _| panic!("{message}")
}

fn inbox_fetch_error(
    message: &'static str,
) -> impl Fn(AccountId, u64, u64) -> DbResult<Vec<MessageEntry>> + Send + Sync + 'static {
    move |_, _, _| Err(DbError::Other(message.into()))
}

// ── map_mempool_error_to_rpc ──

#[test]
fn mempool_full_maps_to_capacity_code() {
    let err = OLMempoolError::MempoolFull {
        current: 100,
        limit: 100,
    };
    assert_eq!(
        map_mempool_error_to_rpc(err).code(),
        MEMPOOL_CAPACITY_ERROR_CODE
    );
}

#[test]
fn byte_limit_exceeded_maps_to_capacity_code() {
    let err = OLMempoolError::MempoolByteLimitExceeded {
        current: 5000,
        limit: 4096,
    };
    assert_eq!(
        map_mempool_error_to_rpc(err).code(),
        MEMPOOL_CAPACITY_ERROR_CODE
    );
}

#[test]
fn account_does_not_exist_maps_to_invalid_params() {
    let err = OLMempoolError::AccountDoesNotExist {
        account: test_account_id(1),
    };
    assert_eq!(map_mempool_error_to_rpc(err).code(), INVALID_PARAMS_CODE);
}

#[test]
fn transaction_too_large_maps_to_invalid_params() {
    let err = OLMempoolError::TransactionTooLarge {
        size: 5000,
        limit: 1000,
    };
    assert_eq!(map_mempool_error_to_rpc(err).code(), INVALID_PARAMS_CODE);
}

#[test]
fn used_sequence_number_maps_to_invalid_params() {
    let err = OLMempoolError::UsedSequenceNumber {
        txid: OLTxId::from(Buf32::zero()),
        expected: 5,
        actual: 4,
    };
    assert_eq!(map_mempool_error_to_rpc(err).code(), INVALID_PARAMS_CODE);
}

#[test]
fn sequence_number_gap_maps_to_invalid_params() {
    let err = OLMempoolError::SequenceNumberGap {
        expected: 1,
        actual: 5,
    };
    assert_eq!(map_mempool_error_to_rpc(err).code(), INVALID_PARAMS_CODE);
}

#[test]
fn database_error_maps_to_internal() {
    let err = OLMempoolError::Database(strata_db_types::DbError::Other("test".into()));
    assert_eq!(map_mempool_error_to_rpc(err).code(), INTERNAL_ERROR_CODE);
}

#[test]
fn service_closed_maps_to_internal() {
    let err = OLMempoolError::ServiceClosed("gone".into());
    assert_eq!(map_mempool_error_to_rpc(err).code(), INTERNAL_ERROR_CODE);
}

#[test]
fn serialization_error_maps_to_internal() {
    let err = OLMempoolError::Serialization("bad bytes".into());
    assert_eq!(map_mempool_error_to_rpc(err).code(), INTERNAL_ERROR_CODE);
}

#[test]
fn state_provider_error_maps_to_internal() {
    let err = OLMempoolError::StateProvider("unavailable".into());
    assert_eq!(map_mempool_error_to_rpc(err).code(), INTERNAL_ERROR_CODE);
}

// ── chain_status ──

#[tokio::test]
async fn chain_status_errors_when_ol_sync_unavailable() {
    let provider = MockProvider::new(); // no sync status
    let rpc = make_rpc(provider);

    let result = rpc.chain_status().await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INTERNAL_ERROR_CODE);
}

#[tokio::test]
async fn chain_status_returns_correct_values() {
    let tip = OLBlockCommitment::new(100, OLBlockId::from(Buf32::from([1u8; 32])));
    let prev = EpochCommitment::new(1, 50, OLBlockId::from(Buf32::from([2u8; 32])));
    let confirmed = EpochCommitment::new(0, 20, OLBlockId::from(Buf32::from([3u8; 32])));
    let finalized = EpochCommitment::new(0, 20, OLBlockId::from(Buf32::from([4u8; 32])));

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(tip, 2, false, prev, confirmed, finalized))
        .with_state_at(tip, genesis_ol_state());
    let rpc = make_rpc(provider);

    let status = rpc.chain_status().await.expect("chain_status");
    assert_eq!(status.tip().slot(), 100);
    assert_eq!(status.tip().epoch(), 2);
    assert!(!status.tip().is_terminal());
    assert_eq!(status.confirmed().epoch(), 0);
    assert_eq!(status.finalized().epoch(), 0);
    assert_eq!(status.finalized().last_slot(), 20);
    assert_eq!(status.latest().epoch(), 1);
    assert_eq!(status.latest().last_slot(), 50);
}

// ── get_checkpoint_info ──

#[tokio::test]
async fn checkpoint_info_returns_none_when_epoch_missing() {
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32]))),
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let result = rpc.get_checkpoint_info(42).await.expect("checkpoint info");
    assert!(result.is_none());
}

#[tokio::test]
async fn checkpoint_info_returns_expected_l1_and_l2_ranges() {
    let prev_terminal = L2BlockCommitment::new(80, fixed_ol_block_id(0x10));

    let first_epoch_block = make_block(85, 2, *prev_terminal.blkid());
    let first_epoch_blkid = first_epoch_block.header().compute_blkid();
    let mid_epoch_block = make_block(90, 2, first_epoch_blkid);
    let mid_epoch_blkid = mid_epoch_block.header().compute_blkid();
    let terminal_block = make_block(100, 2, mid_epoch_blkid);
    let terminal = L2BlockCommitment::new(100, terminal_block.header().compute_blkid());

    let prev_summary = EpochSummary::new(
        1,
        prev_terminal,
        L2BlockCommitment::new(60, fixed_ol_block_id(0x11)),
        L1BlockCommitment::new(500, fixed_l1_block_id(0x30)),
        fixed_buf32(0x40),
    );
    let cur_summary = EpochSummary::new(
        2,
        terminal,
        prev_terminal,
        L1BlockCommitment::new(510, fixed_l1_block_id(0x31)),
        fixed_buf32(0x41),
    );

    let prev_commitment = prev_summary.get_epoch_commitment();
    let cur_commitment = cur_summary.get_epoch_commitment();

    let l1_ref = CheckpointL1Ref::new(
        L1BlockCommitment::new(505, fixed_l1_block_id(0x50)),
        RBuf32::from(fixed_buf32(0xAA).0),
        RBuf32::from(fixed_buf32(0xBB).0),
    );

    let tip = OLBlockCommitment::new(120, fixed_ol_block_id(0x77));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            3,
            false,
            prev_commitment,
            cur_commitment,
            prev_commitment,
        ))
        .with_l1_tip_height(509)
        .with_epoch_commitment(1, prev_commitment)
        .with_epoch_commitment(2, cur_commitment)
        .with_epoch_summary(prev_summary)
        .with_epoch_summary(cur_summary)
        .with_block_and_state(&first_epoch_block, genesis_ol_state())
        .with_block_and_state(&mid_epoch_block, genesis_ol_state())
        .with_block_and_state(&terminal_block, genesis_ol_state())
        .with_manifest(
            AsmManifest::new(
                501,
                L1BlockId::from(Buf32::from([0x61; 32])),
                WtxidsRoot::default(),
                vec![],
            )
            .expect("test manifest should be valid"),
        )
        .with_checkpoint_l1_ref(cur_commitment, l1_ref);

    let rpc = make_rpc(provider);

    let info = rpc
        .get_checkpoint_info(2)
        .await
        .expect("checkpoint info")
        .expect("checkpoint should exist");

    assert_eq!(info.idx, 2);
    assert_eq!(info.l2_range.0.slot(), 85);
    assert_eq!(info.l2_range.1, terminal);
    assert_eq!(info.l1_range.0.height(), 501);
    assert_eq!(info.l1_range.1.height(), 510);
}

#[tokio::test]
async fn checkpoint_info_returns_confirmed_status_with_l1_ref() {
    let prev_terminal = L2BlockCommitment::new(80, fixed_ol_block_id(0x10));
    let observed_height = 505;
    let l1_tip_height = 509;
    let checkpoint_txid = fixed_buf32(0xAA);
    let checkpoint_wtxid = fixed_buf32(0xBB);

    let first_epoch_block = make_block(85, 2, *prev_terminal.blkid());
    let first_epoch_blkid = first_epoch_block.header().compute_blkid();
    let mid_epoch_block = make_block(90, 2, first_epoch_blkid);
    let mid_epoch_blkid = mid_epoch_block.header().compute_blkid();
    let terminal_block = make_block(100, 2, mid_epoch_blkid);
    let terminal = L2BlockCommitment::new(100, terminal_block.header().compute_blkid());

    let prev_summary = EpochSummary::new(
        1,
        prev_terminal,
        L2BlockCommitment::new(60, fixed_ol_block_id(0x11)),
        L1BlockCommitment::new(500, fixed_l1_block_id(0x30)),
        fixed_buf32(0x40),
    );
    let cur_summary = EpochSummary::new(
        2,
        terminal,
        prev_terminal,
        L1BlockCommitment::new(510, fixed_l1_block_id(0x31)),
        fixed_buf32(0x41),
    );

    let prev_commitment = prev_summary.get_epoch_commitment();
    let cur_commitment = cur_summary.get_epoch_commitment();

    let l1_ref = CheckpointL1Ref::new(
        L1BlockCommitment::new(observed_height, fixed_l1_block_id(0x50)),
        RBuf32::from(checkpoint_txid.0),
        RBuf32::from(checkpoint_wtxid.0),
    );

    let tip = OLBlockCommitment::new(120, fixed_ol_block_id(0x77));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            3,
            false,
            prev_commitment,
            cur_commitment,
            prev_commitment,
        ))
        .with_l1_tip_height(l1_tip_height)
        .with_epoch_commitment(1, prev_commitment)
        .with_epoch_commitment(2, cur_commitment)
        .with_epoch_summary(prev_summary)
        .with_epoch_summary(cur_summary)
        .with_block_and_state(&first_epoch_block, genesis_ol_state())
        .with_block_and_state(&mid_epoch_block, genesis_ol_state())
        .with_block_and_state(&terminal_block, genesis_ol_state())
        .with_manifest(
            AsmManifest::new(
                501,
                L1BlockId::from(Buf32::from([0x61; 32])),
                WtxidsRoot::default(),
                vec![],
            )
            .expect("test manifest should be valid"),
        )
        .with_checkpoint_l1_ref(cur_commitment, l1_ref);

    let rpc = make_rpc(provider);

    let info = rpc
        .get_checkpoint_info(2)
        .await
        .expect("checkpoint info")
        .expect("checkpoint should exist");

    match info.confirmation_status {
        RpcCheckpointConfStatus::Confirmed { l1_reference } => {
            assert_eq!(l1_reference.l1_block.height(), observed_height);
            assert_eq!(l1_reference.txid, RBuf32::from(checkpoint_txid.0));
            assert_eq!(l1_reference.wtxid, RBuf32::from(checkpoint_wtxid.0));
        }
        _ => panic!("expected confirmed checkpoint status"),
    }
}

#[tokio::test]
async fn checkpoint_info_returns_pending_when_observation_missing() {
    let prev_terminal = L2BlockCommitment::new(80, fixed_ol_block_id(0x10));
    let first_epoch_block = make_block(85, 2, *prev_terminal.blkid());
    let first_epoch_blkid = first_epoch_block.header().compute_blkid();
    let mid_epoch_block = make_block(90, 2, first_epoch_blkid);
    let mid_epoch_blkid = mid_epoch_block.header().compute_blkid();
    let terminal_block = make_block(100, 2, mid_epoch_blkid);
    let terminal = L2BlockCommitment::new(100, terminal_block.header().compute_blkid());

    let prev_summary = EpochSummary::new(
        1,
        prev_terminal,
        L2BlockCommitment::new(60, fixed_ol_block_id(0x11)),
        L1BlockCommitment::new(500, fixed_l1_block_id(0x30)),
        fixed_buf32(0x40),
    );
    let cur_summary = EpochSummary::new(
        2,
        terminal,
        prev_terminal,
        L1BlockCommitment::new(510, fixed_l1_block_id(0x31)),
        fixed_buf32(0x41),
    );

    let prev_commitment = prev_summary.get_epoch_commitment();
    let cur_commitment = cur_summary.get_epoch_commitment();

    let tip = OLBlockCommitment::new(120, fixed_ol_block_id(0x77));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            3,
            false,
            prev_commitment,
            cur_commitment,
            prev_commitment,
        ))
        .with_l1_tip_height(509)
        .with_epoch_commitment(1, prev_commitment)
        .with_epoch_commitment(2, cur_commitment)
        .with_epoch_summary(prev_summary)
        .with_epoch_summary(cur_summary)
        .with_block_and_state(&first_epoch_block, genesis_ol_state())
        .with_block_and_state(&mid_epoch_block, genesis_ol_state())
        .with_block_and_state(&terminal_block, genesis_ol_state())
        .with_manifest(
            AsmManifest::new(501, fixed_l1_block_id(0x61), WtxidsRoot::default(), vec![])
                .expect("test manifest should be valid"),
        );

    let rpc = make_rpc(provider);

    let info = rpc
        .get_checkpoint_info(2)
        .await
        .expect("checkpoint info")
        .expect("checkpoint should exist");

    assert!(matches!(
        info.confirmation_status,
        RpcCheckpointConfStatus::Pending
    ));
}

#[tokio::test]
async fn checkpoint_info_returns_finalized_status_when_epoch_is_finalized() {
    let prev_terminal = L2BlockCommitment::new(80, fixed_ol_block_id(0x10));
    let observed_height = 505;
    let l1_tip_height = 509;
    let checkpoint_txid = fixed_buf32(0xAA);
    let checkpoint_wtxid = fixed_buf32(0xBB);
    let first_epoch_block = make_block(85, 2, *prev_terminal.blkid());
    let first_epoch_blkid = first_epoch_block.header().compute_blkid();
    let mid_epoch_block = make_block(90, 2, first_epoch_blkid);
    let mid_epoch_blkid = mid_epoch_block.header().compute_blkid();
    let terminal_block = make_block(100, 2, mid_epoch_blkid);
    let terminal = L2BlockCommitment::new(100, terminal_block.header().compute_blkid());

    let prev_summary = EpochSummary::new(
        1,
        prev_terminal,
        L2BlockCommitment::new(60, fixed_ol_block_id(0x11)),
        L1BlockCommitment::new(500, fixed_l1_block_id(0x30)),
        fixed_buf32(0x40),
    );
    let cur_summary = EpochSummary::new(
        2,
        terminal,
        prev_terminal,
        L1BlockCommitment::new(510, fixed_l1_block_id(0x31)),
        fixed_buf32(0x41),
    );

    let prev_commitment = prev_summary.get_epoch_commitment();
    let cur_commitment = cur_summary.get_epoch_commitment();

    let l1_ref = CheckpointL1Ref::new(
        L1BlockCommitment::new(observed_height, fixed_l1_block_id(0x50)),
        RBuf32::from(checkpoint_txid.0),
        RBuf32::from(checkpoint_wtxid.0),
    );

    let tip = OLBlockCommitment::new(120, fixed_ol_block_id(0x77));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            3,
            false,
            prev_commitment,
            cur_commitment,
            cur_commitment,
        ))
        .with_l1_tip_height(l1_tip_height)
        .with_epoch_commitment(1, prev_commitment)
        .with_epoch_commitment(2, cur_commitment)
        .with_epoch_summary(prev_summary)
        .with_epoch_summary(cur_summary)
        .with_block_and_state(&first_epoch_block, genesis_ol_state())
        .with_block_and_state(&mid_epoch_block, genesis_ol_state())
        .with_block_and_state(&terminal_block, genesis_ol_state())
        .with_manifest(
            AsmManifest::new(501, fixed_l1_block_id(0x61), WtxidsRoot::default(), vec![])
                .expect("test manifest should be valid"),
        )
        .with_checkpoint_l1_ref(cur_commitment, l1_ref);

    let rpc = make_rpc(provider);

    let info = rpc
        .get_checkpoint_info(2)
        .await
        .expect("checkpoint info")
        .expect("checkpoint should exist");

    match info.confirmation_status {
        RpcCheckpointConfStatus::Finalized { l1_reference } => {
            assert_eq!(l1_reference.l1_block.height(), observed_height);
            assert_eq!(l1_reference.txid, RBuf32::from(checkpoint_txid.0));
            assert_eq!(l1_reference.wtxid, RBuf32::from(checkpoint_wtxid.0));
        }
        _ => panic!("expected finalized checkpoint status"),
    }
}

#[tokio::test]
async fn checkpoint_info_epoch_0_l1_range_from_genesis() {
    let genesis_blkid = fixed_ol_block_id(0x01);
    let first_block = make_block(1, 0, genesis_blkid);
    let first_blkid = first_block.header().compute_blkid();
    let terminal_block = make_block(10, 0, first_blkid);
    let terminal = L2BlockCommitment::new(10, terminal_block.header().compute_blkid());
    let prev_terminal = L2BlockCommitment::new(0, genesis_blkid);

    let summary = EpochSummary::new(
        0,
        terminal,
        prev_terminal,
        L1BlockCommitment::new(5, fixed_l1_block_id(0x55)),
        fixed_buf32(0x99),
    );
    let commitment = summary.get_epoch_commitment();

    let l1_start_blkid = fixed_l1_block_id(0x71);
    let tip = OLBlockCommitment::new(20, fixed_ol_block_id(0x77));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            1,
            false,
            commitment,
            commitment,
            EpochCommitment::null(),
        ))
        .with_l1_tip_height(10)
        .with_epoch_commitment(0, commitment)
        .with_epoch_summary(summary)
        .with_block_and_state(&first_block, genesis_ol_state())
        .with_block_and_state(&terminal_block, genesis_ol_state())
        .with_manifest(
            AsmManifest::new(
                TEST_GENESIS_L1_HEIGHT + 1,
                l1_start_blkid,
                WtxidsRoot::default(),
                vec![],
            )
            .expect("test manifest should be valid"),
        );

    let rpc = make_rpc(provider);

    let info = rpc
        .get_checkpoint_info(0)
        .await
        .expect("checkpoint info")
        .expect("epoch 0 checkpoint should exist");

    assert_eq!(info.idx, 0);
    assert_eq!(info.l1_range.0.height(), TEST_GENESIS_L1_HEIGHT + 1);
    assert_eq!(*info.l1_range.0.blkid(), l1_start_blkid);
    assert_eq!(info.l1_range.1.height(), 5);
    assert_eq!(info.l2_range.0.slot(), 1);
    assert_eq!(info.l2_range.1, terminal);
}

#[tokio::test]
async fn checkpoint_info_errors_when_l1_tip_is_below_observed_height() {
    let prev_terminal = L2BlockCommitment::new(80, fixed_ol_block_id(0x10));
    let observed_height = 505;
    let checkpoint_txid = fixed_buf32(0xAA);
    let checkpoint_wtxid = fixed_buf32(0xBB);
    let first_epoch_block = make_block(85, 2, *prev_terminal.blkid());
    let first_epoch_blkid = first_epoch_block.header().compute_blkid();
    let mid_epoch_block = make_block(90, 2, first_epoch_blkid);
    let mid_epoch_blkid = mid_epoch_block.header().compute_blkid();
    let terminal_block = make_block(100, 2, mid_epoch_blkid);
    let terminal = L2BlockCommitment::new(100, terminal_block.header().compute_blkid());

    let prev_summary = EpochSummary::new(
        1,
        prev_terminal,
        L2BlockCommitment::new(60, fixed_ol_block_id(0x11)),
        L1BlockCommitment::new(500, fixed_l1_block_id(0x30)),
        fixed_buf32(0x40),
    );
    let cur_summary = EpochSummary::new(
        2,
        terminal,
        prev_terminal,
        L1BlockCommitment::new(510, fixed_l1_block_id(0x31)),
        fixed_buf32(0x41),
    );

    let prev_commitment = prev_summary.get_epoch_commitment();
    let cur_commitment = cur_summary.get_epoch_commitment();

    let l1_ref = CheckpointL1Ref::new(
        L1BlockCommitment::new(observed_height, fixed_l1_block_id(0x50)),
        RBuf32::from(checkpoint_txid.0),
        RBuf32::from(checkpoint_wtxid.0),
    );

    let tip = OLBlockCommitment::new(120, fixed_ol_block_id(0x77));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            3,
            false,
            prev_commitment,
            cur_commitment,
            prev_commitment,
        ))
        .with_l1_tip_height(504)
        .with_epoch_commitment(1, prev_commitment)
        .with_epoch_commitment(2, cur_commitment)
        .with_epoch_summary(prev_summary)
        .with_epoch_summary(cur_summary)
        .with_block_and_state(&first_epoch_block, genesis_ol_state())
        .with_block_and_state(&mid_epoch_block, genesis_ol_state())
        .with_block_and_state(&terminal_block, genesis_ol_state())
        .with_manifest(
            AsmManifest::new(501, fixed_l1_block_id(0x61), WtxidsRoot::default(), vec![])
                .expect("test manifest should be valid"),
        )
        .with_checkpoint_l1_ref(cur_commitment, l1_ref);

    let rpc = make_rpc(provider);

    let result = rpc.get_checkpoint_info(2).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INTERNAL_ERROR_CODE);
}

// ── get_blocks_summaries ──

#[tokio::test]
async fn blocks_summaries_start_gt_end_returns_invalid_params() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        tip,
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let result = rpc.get_blocks_summaries(test_account_id(1), 10, 5).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

#[tokio::test]
async fn blocks_summaries_no_block_at_end_returns_empty() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        tip,
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let result = rpc
        .get_blocks_summaries(test_account_id(1), 0, 99)
        .await
        .expect("should succeed");
    assert!(result.is_empty());
}

#[tokio::test]
async fn blocks_summaries_returns_ascending_order() {
    let account_id = test_account_id(1);

    // Genesis (epoch 0, slot 0) → three blocks in epoch 1 (slots 1..=3
    // under 5-slots-per-epoch sealing).
    let genesis_block = make_block(0, 0, null_blkid());
    let genesis_blkid = genesis_block.header().compute_blkid();
    let prev = EpochCommitment::new(0, 0, genesis_blkid);

    let block0 = make_block(1, 1, genesis_blkid);
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(2, 1, blkid0);
    let blkid1 = block1.header().compute_blkid();
    let block2 = make_block(3, 1, blkid1);
    let blkid2 = block2.header().compute_blkid();

    let tip = OLBlockCommitment::new(3, blkid2);
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            1,
            false,
            prev,
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_epoch_commitment(0, prev)
        .with_block_and_state(&genesis_block, genesis_ol_state())
        .with_block_and_state(
            &block0,
            ol_state_with_snark_account(account_id, 1, 0, DEFAULT_NEXT_INBOX_MSG_IDX),
        )
        .with_block_and_state(
            &block1,
            ol_state_with_snark_account(account_id, 2, 1, DEFAULT_NEXT_INBOX_MSG_IDX),
        )
        .with_block_and_state(
            &block2,
            ol_state_with_snark_account(account_id, 3, 2, DEFAULT_NEXT_INBOX_MSG_IDX),
        );
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 1, 3)
        .await
        .expect("summaries");

    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].block_commitment().slot(), 1);
    assert_eq!(summaries[1].block_commitment().slot(), 2);
    assert_eq!(summaries[2].block_commitment().slot(), 3);
}

#[tokio::test]
async fn blocks_summaries_snark_vs_non_snark() {
    let snark_id = test_account_id(1);
    let empty_id = test_account_id(2);

    let block = make_block(0, 0, null_blkid());
    let blkid = block.header().compute_blkid();

    let snark_state = ol_state_with_snark_account(snark_id, 0, 42, DEFAULT_NEXT_INBOX_MSG_IDX);
    let mut state = MemoryStateBaseLayer::new(snark_state);
    let empty_acct = NewAccountData::new(BitcoinAmount::from(0), NewAccountTypeState::Empty);
    state.create_new_account(empty_id, empty_acct).unwrap();
    let state = state.into_inner();

    let tip = OLBlockCommitment::new(0, blkid);
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, state);
    let rpc = make_rpc(provider);

    let snark = rpc
        .get_blocks_summaries(snark_id, 0, 0)
        .await
        .expect("snark");
    assert_eq!(snark.len(), 1);
    assert_eq!(snark[0].next_seq_no(), 42);

    let empty = rpc
        .get_blocks_summaries(empty_id, 0, 0)
        .await
        .expect("empty");
    assert_eq!(empty.len(), 1);
    assert_eq!(empty[0].next_seq_no(), 0);
    assert_eq!(empty[0].next_inbox_msg_idx(), 0);
}

#[tokio::test]
async fn blocks_summaries_reports_inbox_accumulator_count() {
    let account_id = test_account_id(3);

    let block0 = make_block(0, 0, null_blkid());
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(1, 0, blkid0);
    let blkid1 = block1.header().compute_blkid();

    let tip = OLBlockCommitment::new(1, blkid1);
    let deposit_message = make_message_entry(test_account_id(50), 0, 1_000, vec![0x02, 0xaa]);
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block0, ol_state_with_snark_account(account_id, 0, 0, 0))
        .with_block_and_state(
            &block1,
            ol_state_with_snark_account_and_inbox_entries(
                account_id,
                1,
                0,
                0,
                slice::from_ref(&deposit_message),
            ),
        );
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 1)
        .await
        .expect("summaries");

    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].next_inbox_msg_idx(), 0);
    assert_eq!(summaries[1].next_inbox_msg_idx(), 1);
}

// ── get_blocks_summaries: indexed activity ──

#[tokio::test]
async fn blocks_summaries_empty_index_records_produce_no_activity() {
    let account_id = test_account_id(1);
    let block = make_block(0, 0, null_blkid());
    let tip = OLBlockCommitment::new(0, block.header().compute_blkid());
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 5, 3));
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].updates().is_empty());
    assert!(summaries[0].new_inbox_messages().is_empty());
}

#[tokio::test]
async fn blocks_summaries_populates_updates_and_new_inbox_messages_from_index() {
    let account_id = test_account_id(1);
    let block = make_block(0, 0, null_blkid());
    let commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
    let final_state_root = fixed_buf32(0x66);
    let extra_data = vec![0xF0, 0x0D];
    let update_record = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(commitment, final_state_root)),
        6,
        2,
        Some(extra_data.clone()),
    );
    let inbox_message = make_message_entry(test_account_id(9), 0, 11, vec![0xAA, 0xBB]);
    let inbox_record = InboxMessageRecord::new(inbox_message.as_ssz_bytes(), Some(commitment));

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 7, 2))
        .with_account_update_records(account_id, 0, vec![update_record])
        .with_account_inbox_records(account_id, 0, vec![inbox_record]);
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].updates().len(), 1);

    let update = rpc_update_to_input(summaries[0].updates()[0].clone());
    assert_eq!(update.seq_no(), 6);
    assert_eq!(update.extra_data(), extra_data.as_slice());
    assert_eq!(update.new_state().inner_state(), final_state_root);
    assert_eq!(update.new_state().next_inbox_msg_idx(), 2);
    assert!(update.processed_messages().is_empty());

    let returned_messages = rpc_messages_to_entries(summaries[0].new_inbox_messages());
    assert_eq!(returned_messages.len(), 1);
    assert_eq!(returned_messages[0].source(), inbox_message.source());
    assert_eq!(
        returned_messages[0].incl_epoch(),
        inbox_message.incl_epoch()
    );
    assert_eq!(
        returned_messages[0].payload_value(),
        inbox_message.payload_value()
    );
    assert_eq!(
        returned_messages[0].payload_buf(),
        inbox_message.payload_buf()
    );
}

#[tokio::test]
async fn blocks_summaries_slices_processed_messages_from_index_ranges() {
    let account_id = test_account_id(1);
    let epoch: Epoch = 2;
    let prev_next_inbox_msg_idx = 2;
    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 5, 0x10);

    let block = make_block(10, epoch, null_blkid());
    let commitment = OLBlockCommitment::new(10, block.header().compute_blkid());
    let records = vec![
        AccountUpdateRecord::new(
            Some(AccountUpdateMeta::new(commitment, [0x11; 32].into())),
            21,
            4,
            Some(vec![0xA0]),
        ),
        AccountUpdateRecord::new(
            Some(AccountUpdateMeta::new(commitment, [0x22; 32].into())),
            22,
            6,
            Some(vec![0xA1]),
        ),
    ];
    let msgs_1 = [
        make_message_entry(test_account_id(30), epoch, 3, vec![0x01]),
        make_message_entry(test_account_id(31), epoch, 4, vec![0x02]),
    ];
    let msgs_2 = [
        make_message_entry(test_account_id(32), epoch, 5, vec![0x03]),
        make_message_entry(test_account_id(33), epoch, 6, vec![0x04]),
    ];

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment,
            epoch,
            false,
            prev_epoch_commitment,
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(
            prev_epoch_commitment,
            account_id,
            20,
            prev_next_inbox_msg_idx,
        )
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 10, 22, 6))
        .with_account_update_records(account_id, epoch, records)
        .with_inbox_fetch_fn(inbox_fetch_in_range(
            account_id,
            (prev_next_inbox_msg_idx..)
                .zip(msgs_1.iter().chain(msgs_2.iter()).cloned())
                .collect(),
        ));
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 10, 10)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].updates().len(), 2);

    let u0 = rpc_update_to_input(summaries[0].updates()[0].clone());
    let u1 = rpc_update_to_input(summaries[0].updates()[1].clone());
    assert_eq!(u0.processed_messages(), msgs_1.as_slice());
    assert_eq!(u1.processed_messages(), msgs_2.as_slice());
}

#[tokio::test]
async fn blocks_summaries_walks_cursor_across_epochs() {
    let account_id = test_account_id(1);

    // Genesis (epoch 0, slot 0) → block_e1 (epoch 1's terminal, slot 5)
    // → block_e2 (epoch 2's terminal, slot 10).
    let genesis_block = make_block(0, 0, null_blkid());
    let genesis_blkid = genesis_block.header().compute_blkid();
    let epoch0_commitment = EpochCommitment::new(0, 0, genesis_blkid);

    let block_e1 = make_block(5, 1, genesis_blkid);
    let blkid_e1 = block_e1.header().compute_blkid();
    let commitment_e1 = OLBlockCommitment::new(5, blkid_e1);
    let epoch1_commitment = EpochCommitment::new(1, 5, blkid_e1);

    let block_e2 = make_block(10, 2, blkid_e1);
    let commitment_e2 = OLBlockCommitment::new(10, block_e2.header().compute_blkid());

    let record_e1 = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(commitment_e1, [0x11; 32].into())),
        10,
        2,
        Some(vec![0xA0]),
    );
    let record_e2 = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(commitment_e2, [0x22; 32].into())),
        11,
        4,
        Some(vec![0xA1]),
    );

    let msgs_e1 = [
        make_message_entry(test_account_id(20), 1, 1, vec![0x01]),
        make_message_entry(test_account_id(21), 1, 2, vec![0x02]),
    ];
    let msgs_e2 = [
        make_message_entry(test_account_id(22), 2, 3, vec![0x03]),
        make_message_entry(test_account_id(23), 2, 4, vec![0x04]),
    ];

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment_e2,
            2,
            false,
            epoch1_commitment,
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_epoch_commitment(0, epoch0_commitment)
        .with_epoch_commitment(1, epoch1_commitment)
        .with_block_and_state(
            &genesis_block,
            ol_state_with_snark_account(account_id, 0, 9, 0),
        )
        .with_block_and_state(&block_e1, ol_state_with_snark_account(account_id, 5, 10, 2))
        .with_block_and_state(
            &block_e2,
            ol_state_with_snark_account(account_id, 10, 11, 4),
        )
        .with_account_update_records(account_id, 1, vec![record_e1])
        .with_account_update_records(account_id, 2, vec![record_e2])
        .with_inbox_fetch_fn(inbox_fetch_in_range(
            account_id,
            (0u64..)
                .zip(msgs_e1.iter().chain(msgs_e2.iter()).cloned())
                .collect(),
        ));
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 5, 10)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 2);

    let u_e1 = rpc_update_to_input(summaries[0].updates()[0].clone());
    let u_e2 = rpc_update_to_input(summaries[1].updates()[0].clone());
    assert_eq!(u_e1.processed_messages(), msgs_e1.as_slice());
    assert_eq!(u_e2.processed_messages(), msgs_e2.as_slice());
}

#[tokio::test]
async fn blocks_summaries_seeds_cursor_to_zero_for_new_account() {
    let account_id = test_account_id(1);
    let epoch: Epoch = 1;

    let genesis_block = make_block(0, 0, null_blkid());
    let genesis_blkid = genesis_block.header().compute_blkid();
    let prev_epoch_commitment = EpochCommitment::new(0, 0, genesis_blkid);

    let block = make_block(5, epoch, genesis_blkid);
    let commitment = OLBlockCommitment::new(5, block.header().compute_blkid());
    let record = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(commitment, [0x33; 32].into())),
        1,
        2,
        Some(vec![0xB0]),
    );
    let msgs = [
        make_message_entry(test_account_id(40), epoch, 1, vec![0x01]),
        make_message_entry(test_account_id(41), epoch, 2, vec![0x02]),
    ];

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment,
            epoch,
            false,
            prev_epoch_commitment,
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_epoch_commitment(0, prev_epoch_commitment)
        // Genesis state has no snark account — exercises the cursor=0 fallback.
        .with_block_and_state(&genesis_block, genesis_ol_state())
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 5, 1, 2))
        .with_account_update_records(account_id, epoch, vec![record])
        .with_inbox_fetch_fn(inbox_fetch_in_range(
            account_id,
            (0u64..).zip(msgs.iter().cloned()).collect(),
        ));
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 5, 5)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 1);

    let update = rpc_update_to_input(summaries[0].updates()[0].clone());
    assert_eq!(update.processed_messages(), msgs.as_slice());
}

#[tokio::test]
async fn blocks_summaries_account_appears_midway_through_range() {
    // Sealing convention: epoch 0 is just genesis (slot 0); each subsequent
    // epoch has 5 slots, epoch K terminating at slot 5K. Test data uses one
    // block per non-genesis epoch, placed at the epoch's terminal slot.
    const SLOTS_PER_EPOCH: u64 = 5;
    const NUM_EPOCHS: u32 = 5;
    const APPEARS_AT_EPOCH: Epoch = 3;

    let account_id = test_account_id(1);

    // Genesis block (epoch 0, slot 0). Subsequent epoch-1..=5 blocks chain
    // their parent links back through it.
    let genesis_block = make_block(0, 0, null_blkid());
    let genesis_blkid = genesis_block.header().compute_blkid();
    let epoch0_commitment = EpochCommitment::new(0, 0, genesis_blkid);

    // Range covers epochs 1..=5. Account doesn't exist at epoch 0 (cursor
    // seeds to 0), has no records in epochs 1 and 2 (cursor stays at 0),
    // first appears at epoch 3, then no records again in epochs 4 and 5.
    // The cursor walk must hold the seed through empty leading epochs,
    // apply [0, next_inbox_idx) to the first record, and emit nothing for
    // the trailing empty epochs.
    let blocks: Vec<OLBlock> = {
        let mut acc = Vec::with_capacity(NUM_EPOCHS as usize);
        let mut parent = genesis_blkid;
        for epoch in 1..=NUM_EPOCHS {
            let slot = u64::from(epoch) * SLOTS_PER_EPOCH;
            let block = make_block(slot, epoch, parent);
            parent = block.header().compute_blkid();
            acc.push(block);
        }
        acc
    };
    let block_for_epoch = |epoch: Epoch| &blocks[(epoch - 1) as usize];
    let appears_block = block_for_epoch(APPEARS_AT_EPOCH);
    let appears_commitment = OLBlockCommitment::new(
        appears_block.header().slot(),
        appears_block.header().compute_blkid(),
    );
    let tip_block = block_for_epoch(NUM_EPOCHS);
    let tip_commitment = OLBlockCommitment::new(
        tip_block.header().slot(),
        tip_block.header().compute_blkid(),
    );

    let record = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(
            appears_commitment,
            [0x33; 32].into(),
        )),
        1,
        2,
        Some(vec![0xA1]),
    );
    let msgs = [
        make_message_entry(test_account_id(40), APPEARS_AT_EPOCH, 1, vec![0x01]),
        make_message_entry(test_account_id(41), APPEARS_AT_EPOCH, 2, vec![0x02]),
    ];

    let provider = (1..=NUM_EPOCHS).zip(blocks.iter()).fold(
        MockProvider::new()
            .with_sync_status(make_sync_status(
                tip_commitment,
                NUM_EPOCHS,
                false,
                EpochCommitment::null(),
                EpochCommitment::null(),
                EpochCommitment::null(),
            ))
            .with_epoch_commitment(0, epoch0_commitment)
            // Genesis block — epoch 0's terminal. Account doesn't exist here.
            .with_block_and_state(&genesis_block, genesis_ol_state())
            .with_account_update_records(account_id, APPEARS_AT_EPOCH, vec![record])
            .with_inbox_fetch_fn(inbox_fetch_in_range(
                account_id,
                (0u64..).zip(msgs.iter().cloned()).collect(),
            )),
        |p, (epoch, block)| {
            let (seqno, idx) = if epoch >= APPEARS_AT_EPOCH {
                (1, 2)
            } else {
                (0, 0)
            };
            p.with_block_and_state(
                block,
                ol_state_with_snark_account(account_id, block.header().slot(), seqno, idx),
            )
        },
    );
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(
            account_id,
            block_for_epoch(1).header().slot(),
            tip_commitment.slot(),
        )
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), NUM_EPOCHS as usize);
    for (epoch, summary) in (1..=NUM_EPOCHS).zip(summaries.iter()) {
        if epoch == APPEARS_AT_EPOCH {
            assert_eq!(
                summary.updates().len(),
                1,
                "epoch {epoch} (account appears)"
            );
            let update = rpc_update_to_input(summary.updates()[0].clone());
            assert_eq!(update.processed_messages(), msgs.as_slice());
        } else {
            assert!(
                summary.updates().is_empty(),
                "empty epoch {epoch} should have no updates",
            );
        }
    }
}

#[tokio::test]
async fn blocks_summaries_ignores_records_for_other_blocks() {
    let account_id = test_account_id(1);
    let block = make_block(0, 0, null_blkid());
    let commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
    let other_commitment = OLBlockCommitment::new(99, fixed_ol_block_id(0x99));
    let update_record = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(other_commitment, [0x44; 32].into())),
        1,
        1,
        Some(vec![0x01]),
    );
    let message = make_message_entry(test_account_id(8), 0, 1, vec![0x08]);
    let inbox_record = InboxMessageRecord::new(message.as_ssz_bytes(), Some(other_commitment));

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 1, 1))
        .with_account_update_records(account_id, 0, vec![update_record])
        .with_account_inbox_records(account_id, 0, vec![inbox_record]);
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].updates().is_empty());
    assert!(summaries[0].new_inbox_messages().is_empty());
}

#[tokio::test]
async fn blocks_summaries_out_of_chain_directset_does_not_fail_rpc() {
    let account_id = test_account_id(1);
    let block = make_block(0, 0, null_blkid());
    let queried_commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
    let other_commitment = OLBlockCommitment::new(99, fixed_ol_block_id(0x99));

    // Out-of-chain DirectSet record (extra_data = None). If filtering happened
    // after hydration, this would trip the "no extra_data (DirectSet)" error
    // and fail the entire RPC. The chain filter must drop it before hydration.
    let out_of_chain_directset = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(other_commitment, [0x99; 32].into())),
        1,
        2,
        None,
    );
    let in_chain_update = AccountUpdateRecord::new(
        Some(AccountUpdateMeta::new(
            queried_commitment,
            [0x11; 32].into(),
        )),
        2,
        4,
        Some(vec![0xA0]),
    );

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            queried_commitment,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 2, 4))
        .with_account_update_records(account_id, 0, vec![out_of_chain_directset, in_chain_update]);
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("RPC must succeed despite out-of-chain DirectSet record");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].updates().len(), 1);

    let update = rpc_update_to_input(summaries[0].updates()[0].clone());
    assert_eq!(update.seq_no(), 2);
    assert_eq!(update.extra_data(), [0xA0]);
}

#[tokio::test]
async fn blocks_summaries_cursor_passes_checkpoint_sync_row() {
    let account_id = test_account_id(1);
    let epoch: Epoch = 2;
    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 5, 0x10);
    let prev_next_inbox_msg_idx = 2;

    let block = make_block(10, epoch, null_blkid());
    let commitment = OLBlockCommitment::new(10, block.header().compute_blkid());

    // Three records in this epoch:
    //   A: in-chain Update consuming inbox [2, 4)
    //   B: checkpoint-sync row (meta=None) consuming inbox [4, 6) — must
    //      advance the cursor without being emitted
    //   C: in-chain Update consuming inbox [6, 8) — its slice depends on the
    //      cursor having moved past B
    let records = vec![
        AccountUpdateRecord::new(
            Some(AccountUpdateMeta::new(commitment, [0x11; 32].into())),
            21,
            4,
            Some(vec![0xA0]),
        ),
        AccountUpdateRecord::new(None, 22, 6, Some(vec![0xA1])),
        AccountUpdateRecord::new(
            Some(AccountUpdateMeta::new(commitment, [0x33; 32].into())),
            23,
            8,
            Some(vec![0xA2]),
        ),
    ];

    let msgs_a = [
        make_message_entry(test_account_id(30), epoch, 1, vec![0x01]),
        make_message_entry(test_account_id(31), epoch, 2, vec![0x02]),
    ];
    let msgs_b_skipped = [
        make_message_entry(test_account_id(32), epoch, 3, vec![0x03]),
        make_message_entry(test_account_id(33), epoch, 4, vec![0x04]),
    ];
    let msgs_c = [
        make_message_entry(test_account_id(34), epoch, 5, vec![0x05]),
        make_message_entry(test_account_id(35), epoch, 6, vec![0x06]),
    ];

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment,
            epoch,
            false,
            prev_epoch_commitment,
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(
            prev_epoch_commitment,
            account_id,
            20,
            prev_next_inbox_msg_idx,
        )
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 10, 23, 8))
        .with_account_update_records(account_id, epoch, records)
        .with_inbox_fetch_fn(inbox_fetch_in_range(
            account_id,
            (prev_next_inbox_msg_idx..)
                .zip(
                    msgs_a
                        .iter()
                        .chain(msgs_b_skipped.iter())
                        .chain(msgs_c.iter())
                        .cloned(),
                )
                .collect(),
        ));
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 10, 10)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].updates().len(), 2);

    let u_a = rpc_update_to_input(summaries[0].updates()[0].clone());
    let u_c = rpc_update_to_input(summaries[0].updates()[1].clone());
    assert_eq!(u_a.processed_messages(), msgs_a.as_slice());
    assert_eq!(u_c.processed_messages(), msgs_c.as_slice());
}

// ── get_blocks_summaries: property tests over indexed records ──

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]

    #[test]
    fn blocks_summaries_groups_index_updates_by_block(
        generated_updates in prop::collection::vec(
            (
                any::<bool>(),
                any::<u16>(),
                any::<[u8; 32]>(),
                prop::collection::vec(any::<u8>(), 0..16)
            ),
            0..16
        )
    ) {
        let account_id = test_account_id(1);
        // Genesis (epoch 0, slot 0) → two blocks in epoch 1 (slots 1 and 2
        // under 5-slots-per-epoch sealing).
        let epoch: Epoch = 1;
        let genesis_block = make_block(0, 0, null_blkid());
        let genesis_blkid = genesis_block.header().compute_blkid();
        let block0 = make_block(1, epoch, genesis_blkid);
        let block0_commitment = OLBlockCommitment::new(1, block0.header().compute_blkid());
        let block1 = make_block(2, epoch, block0.header().compute_blkid());
        let block1_commitment = OLBlockCommitment::new(2, block1.header().compute_blkid());

        // Property under test is grouping by block_commitment, not cursor walking.
        // Force `next_inbox_idx = 0` for every record so the cursor never advances
        // and the inbox fetch is skipped — keeps `processed_messages` empty
        // regardless of record ordering.
        let records: Vec<AccountUpdateRecord> = generated_updates
            .iter()
            .map(|(use_second_block, seq_no, root, extra_data)| {
                let commitment = if *use_second_block {
                    block1_commitment
                } else {
                    block0_commitment
                };
                AccountUpdateRecord::new(
                    Some(AccountUpdateMeta::new(commitment, (*root).into())),
                    u64::from(*seq_no),
                    0,
                    Some(extra_data.clone()),
                )
            })
            .collect();

        let prev_epoch_commitment = EpochCommitment::new(0, 0, genesis_blkid);
        let provider = MockProvider::new()
            .with_sync_status(make_sync_status(block1_commitment, epoch, false, EpochCommitment::null(), EpochCommitment::null(), EpochCommitment::null()))
            .with_epoch_commitment(0, prev_epoch_commitment)
            .with_block_and_state(&genesis_block, genesis_ol_state())
            .with_block_and_state(&block0, ol_state_with_snark_account(account_id, 1, 99, 99))
            .with_block_and_state(&block1, ol_state_with_snark_account(account_id, 2, 99, 99))
            .with_account_update_records(account_id, epoch, records);
        let rpc = make_rpc(provider);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        let summaries = rt
            .block_on(async { rpc.get_blocks_summaries(account_id, 1, 2).await })
            .expect("summaries");
        prop_assert_eq!(summaries.len(), 2);

        let expected_for_block = |use_second_block: bool| {
            generated_updates
                .iter()
                .filter(move |(is_second, _, _, _)| *is_second == use_second_block)
                .collect::<Vec<_>>()
        };
        let expected_block0 = expected_for_block(false);
        let expected_block1 = expected_for_block(true);
        prop_assert_eq!(summaries[0].updates().len(), expected_block0.len());
        prop_assert_eq!(summaries[1].updates().len(), expected_block1.len());

        for (rpc_update, (_, seq_no, root, extra_data)) in
            summaries[0].updates().iter().zip(expected_block0.iter())
        {
            let update = rpc_update_to_input(rpc_update.clone());
            prop_assert_eq!(update.seq_no(), u64::from(*seq_no));
            prop_assert_eq!(update.extra_data(), extra_data.as_slice());
            prop_assert_eq!(
                update.new_state().inner_state(),
                (*root).into()
            );
            prop_assert_eq!(update.new_state().next_inbox_msg_idx(), 0);
            prop_assert!(update.processed_messages().is_empty());
        }

        for (rpc_update, (_, seq_no, root, extra_data)) in
            summaries[1].updates().iter().zip(expected_block1.iter())
        {
            let update = rpc_update_to_input(rpc_update.clone());
            prop_assert_eq!(update.seq_no(), u64::from(*seq_no));
            prop_assert_eq!(update.extra_data(), extra_data.as_slice());
            prop_assert_eq!(
                update.new_state().inner_state(),
                (*root).into()
            );
            prop_assert_eq!(update.new_state().next_inbox_msg_idx(), 0);
            prop_assert!(update.processed_messages().is_empty());
        }
    }
}

// ── get_acct_epoch_summary ──

#[tokio::test]
async fn epoch_summary_nonexistent_epoch_errors() {
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32]))),
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let result = rpc.get_acct_epoch_summary(test_account_id(1), 99).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn epoch_summary_nonexistent_account_errors() {
    let block = make_block(10, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(10, blkid);
    let epoch_commit = EpochCommitment::new(0, 10, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            terminal,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, genesis_ol_state())
        .with_epoch_commitment(0, epoch_commit);
    let rpc = make_rpc(provider);

    let result = rpc.get_acct_epoch_summary(test_account_id(99), 0).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

#[tokio::test]
async fn epoch_summary_valid_snark_account() {
    let account_id = test_account_id(1);

    let block = make_block(20, 1, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(20, blkid);

    let prev_blkid = OLBlockId::from(Buf32::from([1u8; 32]));
    let epoch1_commit = EpochCommitment::new(1, 20, blkid);
    let epoch0_commit = EpochCommitment::new(0, 10, prev_blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            terminal,
            1,
            false,
            epoch0_commit,
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(
            &block,
            ol_state_with_snark_account(account_id, 20, 5, DEFAULT_NEXT_INBOX_MSG_IDX),
        )
        .with_epoch_commitment(1, epoch1_commit)
        .with_epoch_commitment(0, epoch0_commit);
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, 1)
        .await
        .expect("epoch summary");

    assert_eq!(summary.epoch_commitment().epoch(), 1);
    assert_eq!(summary.prev_epoch_commitment().epoch(), 0);
    assert_eq!(summary.balance(), 0);
}

#[tokio::test]
async fn epoch_summary_epoch_zero_null_prev() {
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(5, blkid);
    let epoch0_commit = EpochCommitment::new(0, 5, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            terminal,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(
            &block,
            ol_state_with_snark_account(account_id, 5, 0, DEFAULT_NEXT_INBOX_MSG_IDX),
        )
        .with_epoch_commitment(0, epoch0_commit);
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, 0)
        .await
        .expect("epoch 0");
    assert_eq!(summary.prev_epoch_commitment().epoch(), 0);
    assert_eq!(summary.prev_epoch_commitment().last_slot(), 0);
}

#[tokio::test]
async fn epoch_summary_non_snark_account() {
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(5, blkid);
    let epoch0_commit = EpochCommitment::new(0, 5, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            terminal,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_empty_account(account_id, 5))
        .with_epoch_commitment(0, epoch0_commit);
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, 0)
        .await
        .expect("non-snark");
    assert_eq!(summary.balance(), 0);
    assert!(summary.update_inputs().is_empty());
}

#[tokio::test]
async fn epoch_summary_returns_messages_from_mmr_range() {
    let epoch = 2;
    let account_id = test_account_id(11);
    let prev_next_inbox_msg_idx = 2;
    let cur_next_inbox_msg_idx = 5;
    let prev_seq_no = 6;
    let cur_seq_no = 7;

    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 30, 0x61);
    let epoch_commitment = test_epoch_commitment(epoch, 40, 0x62);
    let expected_messages = vec![
        make_message_entry(test_account_id(50), epoch, 3, vec![2, 6]),
        make_message_entry(test_account_id(51), epoch, 4, vec![3, 9]),
        make_message_entry(test_account_id(52), epoch, 5, vec![4, 12]),
    ];

    let provider = MockProvider::new()
        .with_epoch_commitment(epoch, epoch_commitment)
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(
            epoch_commitment,
            account_id,
            cur_seq_no,
            cur_next_inbox_msg_idx,
        )
        .with_snark_state_at_terminal(
            prev_epoch_commitment,
            account_id,
            prev_seq_no,
            prev_next_inbox_msg_idx,
        )
        .with_account_extra_data_at_terminal(
            account_id,
            epoch,
            cur_seq_no,
            cur_next_inbox_msg_idx,
            vec![2, 2, 5],
            epoch_commitment,
        )
        .with_inbox_fetch_fn(inbox_fetch_expect_success(
            account_id,
            prev_next_inbox_msg_idx,
            cur_next_inbox_msg_idx,
            expected_messages.clone(),
        ));
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, epoch)
        .await
        .expect("epoch summary");
    let update = summary.update_inputs().first().expect("update input");
    let returned_messages = rpc_messages_to_entries(&update.messages);

    assert_eq!(returned_messages.len(), expected_messages.len());
    for (actual, expected) in returned_messages.iter().zip(expected_messages.iter()) {
        assert_eq!(actual.source(), expected.source());
        assert_eq!(actual.incl_epoch(), expected.incl_epoch());
        assert_eq!(actual.payload_value(), expected.payload_value());
        assert_eq!(actual.payload_buf(), expected.payload_buf());
    }
}

#[tokio::test]
async fn epoch_summary_multi_record_slices_messages_per_update() {
    // Three updates in one epoch with frontier progression 2 -> 4 -> 4 -> 7.
    // Update 1 consumes messages [2,4), update 2 consumes nothing (idx unchanged),
    // update 3 consumes [4,7). Each update gets its own message list and its own
    // post-update proof state.
    let epoch = 5;
    let account_id = test_account_id(20);
    let prev_next_inbox_msg_idx = 2;

    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 100, 0x70);
    let epoch_commitment = test_epoch_commitment(epoch, 110, 0x71);

    let block1 = OLBlockCommitment::new(105, OLBlockId::from(Buf32::from([0x71; 32])));
    let block2 = OLBlockCommitment::new(108, OLBlockId::from(Buf32::from([0x72; 32])));
    let block3 = OLBlockCommitment::new(110, OLBlockId::from(Buf32::from([0x73; 32])));

    let msgs_1 = vec![
        make_message_entry(test_account_id(60), epoch, 2, vec![0xAA]),
        make_message_entry(test_account_id(61), epoch, 3, vec![0xBB]),
    ];
    let msgs_3 = vec![
        make_message_entry(test_account_id(62), epoch, 4, vec![0xCC]),
        make_message_entry(test_account_id(63), epoch, 5, vec![0xDD]),
        make_message_entry(test_account_id(64), epoch, 6, vec![0xEE]),
    ];

    let records = vec![
        AccountUpdateRecord::new(
            Some(AccountUpdateMeta::new(block1, [0x11; 32].into())),
            10,
            4,
            Some(vec![1, 2]),
        ),
        AccountUpdateRecord::new(
            Some(AccountUpdateMeta::new(block2, [0x22; 32].into())),
            11,
            4,
            Some(vec![3, 4]),
        ),
        AccountUpdateRecord::new(
            Some(AccountUpdateMeta::new(block3, [0x33; 32].into())),
            12,
            7,
            Some(vec![5, 6]),
        ),
    ];

    let provider = MockProvider::new()
        .with_epoch_commitment(epoch, epoch_commitment)
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(epoch_commitment, account_id, 12, 7)
        .with_snark_state_at_terminal(
            prev_epoch_commitment,
            account_id,
            9,
            prev_next_inbox_msg_idx,
        )
        .with_account_update_records(account_id, epoch, records)
        .with_inbox_fetch_fn(inbox_fetch_expect_success(
            account_id,
            2,
            7,
            msgs_1
                .iter()
                .chain(msgs_3.iter())
                .cloned()
                .collect::<Vec<_>>(),
        ));
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, epoch)
        .await
        .expect("epoch summary");
    let updates = summary.update_inputs();
    assert_eq!(updates.len(), 3);

    assert_eq!(updates[0].seq_no, 10);
    assert_eq!(updates[0].messages.len(), 2);
    assert_eq!(rpc_messages_to_entries(&updates[0].messages), msgs_1);
    assert_eq!(updates[1].seq_no, 11);
    assert!(updates[1].messages.is_empty());
    assert_eq!(updates[2].seq_no, 12);
    assert_eq!(updates[2].messages.len(), 3);
    assert_eq!(rpc_messages_to_entries(&updates[2].messages), msgs_3);
}

#[tokio::test]
async fn epoch_summary_epoch_zero_has_no_messages() {
    let epoch = 0;
    let account_id = test_account_id(12);
    let cur_next_inbox_msg_idx = 3;
    let cur_seq_no = 3;

    let epoch_commitment = test_epoch_commitment(epoch, 10, 0x63);
    let provider = MockProvider::new()
        .with_epoch_commitment(epoch, epoch_commitment)
        .with_snark_state_at_terminal(
            epoch_commitment,
            account_id,
            cur_seq_no,
            cur_next_inbox_msg_idx,
        )
        .with_account_extra_data_at_terminal(
            account_id,
            epoch,
            cur_seq_no,
            cur_next_inbox_msg_idx,
            vec![0, 3],
            epoch_commitment,
        )
        .with_inbox_fetch_fn(inbox_fetch_panic("epoch 0 should not fetch inbox messages"));
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, epoch)
        .await
        .expect("epoch summary");
    let update = summary.update_inputs().first().expect("update input");
    assert!(update.messages.is_empty());
}

#[tokio::test]
async fn epoch_summary_no_idx_delta_returns_empty_messages() {
    let epoch = 4;
    let account_id = test_account_id(13);
    let unchanged_next_inbox_msg_idx = 7;
    let prev_seq_no = 8;
    let cur_seq_no = 9;

    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 50, 0x64);
    let epoch_commitment = test_epoch_commitment(epoch, 60, 0x65);

    let provider = MockProvider::new()
        .with_epoch_commitment(epoch, epoch_commitment)
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(
            epoch_commitment,
            account_id,
            cur_seq_no,
            unchanged_next_inbox_msg_idx,
        )
        .with_snark_state_at_terminal(
            prev_epoch_commitment,
            account_id,
            prev_seq_no,
            unchanged_next_inbox_msg_idx,
        )
        .with_account_extra_data_at_terminal(
            account_id,
            epoch,
            cur_seq_no,
            unchanged_next_inbox_msg_idx,
            vec![4, 7],
            epoch_commitment,
        )
        .with_inbox_fetch_fn(inbox_fetch_expect_success(
            account_id,
            unchanged_next_inbox_msg_idx,
            unchanged_next_inbox_msg_idx,
            Vec::new(),
        ));
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, epoch)
        .await
        .expect("epoch summary");
    let update = summary.update_inputs().first().expect("update input");
    assert!(update.messages.is_empty());
}

#[tokio::test]
async fn epoch_summary_account_missing_in_prev_state_starts_from_zero() {
    let epoch = 3;
    let account_id = test_account_id(14);
    let cur_next_inbox_msg_idx = 2;

    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 20, 0x66);
    let epoch_commitment = test_epoch_commitment(epoch, 30, 0x67);
    let expected_messages = vec![
        make_message_entry(test_account_id(50), epoch, 1, vec![0, 0]),
        make_message_entry(test_account_id(51), epoch, 2, vec![1, 3]),
    ];

    let provider = MockProvider::new()
        .with_epoch_commitment(epoch, epoch_commitment)
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(epoch_commitment, account_id, 4, cur_next_inbox_msg_idx)
        .with_genesis_state_at_terminal(prev_epoch_commitment)
        .with_account_extra_data_at_terminal(
            account_id,
            epoch,
            4,
            cur_next_inbox_msg_idx,
            vec![3],
            epoch_commitment,
        )
        .with_inbox_fetch_fn(inbox_fetch_expect_success(
            account_id,
            0,
            cur_next_inbox_msg_idx,
            expected_messages.clone(),
        ));
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, epoch)
        .await
        .expect("epoch summary");
    let update = summary.update_inputs().first().expect("update input");
    let returned_messages = rpc_messages_to_entries(&update.messages);

    assert_eq!(returned_messages.len(), expected_messages.len());
    for (actual, expected) in returned_messages.iter().zip(expected_messages.iter()) {
        assert_eq!(actual.source(), expected.source());
        assert_eq!(actual.incl_epoch(), expected.incl_epoch());
        assert_eq!(actual.payload_value(), expected.payload_value());
        assert_eq!(actual.payload_buf(), expected.payload_buf());
    }
}

#[tokio::test]
async fn epoch_summary_without_extra_data_skips_inbox_fetch() {
    let epoch = 2;
    let account_id = test_account_id(15);
    let cur_next_inbox_msg_idx = 3;
    let prev_next_inbox_msg_idx = 1;

    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 30, 0x68);
    let epoch_commitment = test_epoch_commitment(epoch, 40, 0x69);

    let provider = MockProvider::new()
        .with_epoch_commitment(epoch, epoch_commitment)
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(epoch_commitment, account_id, 8, cur_next_inbox_msg_idx)
        .with_snark_state_at_terminal(
            prev_epoch_commitment,
            account_id,
            7,
            prev_next_inbox_msg_idx,
        )
        .with_inbox_fetch_fn(inbox_fetch_panic(
            "inbox fetch should be skipped when account extra data is absent",
        ));
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, epoch)
        .await
        .expect("epoch summary");
    assert!(summary.update_inputs().is_empty());
}

#[tokio::test]
async fn epoch_summary_mmr_fetch_error_propagates() {
    let epoch = 1;
    let account_id = test_account_id(16);
    let prev_next_inbox_msg_idx = 0;
    let cur_next_inbox_msg_idx = 2;

    let prev_epoch_commitment = test_epoch_commitment(epoch - 1, 10, 0x6A);
    let epoch_commitment = test_epoch_commitment(epoch, 20, 0x6B);

    let provider = MockProvider::new()
        .with_epoch_commitment(epoch, epoch_commitment)
        .with_epoch_commitment(epoch - 1, prev_epoch_commitment)
        .with_snark_state_at_terminal(epoch_commitment, account_id, 4, cur_next_inbox_msg_idx)
        .with_snark_state_at_terminal(
            prev_epoch_commitment,
            account_id,
            3,
            prev_next_inbox_msg_idx,
        )
        .with_account_extra_data_at_terminal(
            account_id,
            epoch,
            4,
            cur_next_inbox_msg_idx,
            vec![1, 0x10],
            epoch_commitment,
        )
        .with_inbox_fetch_fn(inbox_fetch_error("forced inbox fetch failure"));
    let rpc = make_rpc(provider);

    let result = rpc.get_acct_epoch_summary(account_id, epoch).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INTERNAL_ERROR_CODE);
}

// ── submit_transaction ──

#[tokio::test]
async fn submit_transaction_generic_message_succeeds() {
    let account_id = test_account_id(1);
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32]))),
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let tx = make_gam_rpc_tx(account_id, vec![1, 2, 3, 4]);
    let txid = rpc
        .submit_transaction(tx)
        .await
        .expect("submit_transaction");

    assert_ne!(txid, OLTxId::from(Buf32::zero()));
}

#[tokio::test]
async fn submit_transaction_invalid_snark_update_returns_invalid_params() {
    let account_id = test_account_id(1);
    // The RPC layer rejects malformed payloads before calling the provider,
    // so submit_behavior doesn't matter here.
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32]))),
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let bad_tx = RpcOLTransaction::new_snark_acct_update(RpcSnarkAccountUpdate::new(
        HexBytes32::from(*account_id.inner()),
        HexBytes(vec![0xDE, 0xAD]),
        HexBytes(vec![]),
    ));

    let result = rpc.submit_transaction(bad_tx).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

#[tokio::test]
async fn submit_transaction_nonexistent_account_returns_error() {
    let missing = test_account_id(99);
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32]))),
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_submit_fn(move |_| Err(OLMempoolError::AccountDoesNotExist { account: missing }));
    let rpc = make_rpc(provider);

    let tx = make_gam_rpc_tx(missing, vec![1, 2, 3]);
    let result = rpc.submit_transaction(tx).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

// ── get_snark_account_state ──

#[tokio::test]
async fn snark_account_state_latest_returns_state() {
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(5, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(
            &block,
            ol_state_with_snark_account(account_id, 5, 7, DEFAULT_NEXT_INBOX_MSG_IDX),
        );
    let rpc = make_rpc(provider);

    let state = rpc
        .get_snark_account_state(account_id, OLBlockOrTag::Latest)
        .await
        .expect("snark state")
        .expect("should be Some");

    assert_eq!(state.seq_no(), 7);
    assert_eq!(state.next_inbox_msg_idx(), 0);
}

#[tokio::test]
async fn snark_account_state_by_slot() {
    let account_id = test_account_id(1);

    let block = make_block(10, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(10, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(
            &block,
            ol_state_with_snark_account(account_id, 10, 3, DEFAULT_NEXT_INBOX_MSG_IDX),
        );
    let rpc = make_rpc(provider);

    let state = rpc
        .get_snark_account_state(account_id, OLBlockOrTag::Slot(10))
        .await
        .expect("snark state")
        .expect("should be Some");

    assert_eq!(state.seq_no(), 3);
}

#[tokio::test]
async fn snark_account_state_non_snark_returns_none() {
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(5, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_empty_account(account_id, 5));
    let rpc = make_rpc(provider);

    let result = rpc
        .get_snark_account_state(account_id, OLBlockOrTag::Latest)
        .await
        .expect("should succeed");

    assert!(result.is_none());
}

#[tokio::test]
async fn snark_account_state_missing_account_returns_none() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_state_at(tip, genesis_ol_state());
    let rpc = make_rpc(provider);

    let result = rpc
        .get_snark_account_state(test_account_id(99), OLBlockOrTag::Latest)
        .await
        .expect("should succeed");

    assert!(result.is_none());
}

#[tokio::test]
async fn snark_account_state_no_ol_sync_returns_error() {
    let provider = MockProvider::new(); // no sync status
    let rpc = make_rpc(provider);

    let result = rpc
        .get_snark_account_state(test_account_id(1), OLBlockOrTag::Latest)
        .await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INTERNAL_ERROR_CODE);
}

#[tokio::test]
async fn snark_account_state_by_block_id() {
    let account_id = test_account_id(1);

    let block = make_block(8, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(8, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(
            &block,
            ol_state_with_snark_account(account_id, 8, 11, DEFAULT_NEXT_INBOX_MSG_IDX),
        );
    let rpc = make_rpc(provider);

    let state = rpc
        .get_snark_account_state(account_id, OLBlockOrTag::OLBlockId(blkid))
        .await
        .expect("snark state")
        .expect("should be Some");

    assert_eq!(state.seq_no(), 11);
}

// ── get_raw_blocks_range ──

#[tokio::test]
async fn raw_blocks_range_returns_blocks_in_order() {
    let block0 = make_block(0, 0, null_blkid());
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(1, 0, blkid0);
    let blkid1 = block1.header().compute_blkid();
    let block2 = make_block(2, 0, blkid1);
    let blkid2 = block2.header().compute_blkid();

    let tip = OLBlockCommitment::new(2, blkid2);
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block0, genesis_ol_state())
        .with_block_and_state(&block1, genesis_ol_state())
        .with_block_and_state(&block2, genesis_ol_state());
    let rpc = make_rpc(provider);

    let entries = rpc.get_raw_blocks_range(0, 2).await.expect("blocks");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].slot(), 0);
    assert_eq!(entries[1].slot(), 1);
    assert_eq!(entries[2].slot(), 2);
    assert_eq!(entries[0].blkid(), blkid0);
}

#[tokio::test]
async fn raw_blocks_range_start_gt_end_returns_invalid_params() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        tip,
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let result = rpc.get_raw_blocks_range(10, 5).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

#[tokio::test]
async fn raw_blocks_range_exceeds_max_returns_invalid_params() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        tip,
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    // MAX_RAW_BLOCKS_RANGE is 5000, request 5001
    let result = rpc.get_raw_blocks_range(0, 5000).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

// ── get_block_by_slot ──

#[tokio::test]
async fn get_block_by_slot_returns_decoded_detail() {
    let block = make_block(7, 1, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(7, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            1,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, genesis_ol_state());
    let rpc = make_rpc(provider);

    let detail = rpc
        .get_block_by_slot(7)
        .await
        .expect("rpc call")
        .expect("block present");
    assert_eq!(detail.header().slot(), 7);
    assert_eq!(detail.header().epoch(), 1);
    assert_eq!(detail.header().blkid(), blkid);
    assert_eq!(detail.tx_count(), 0);
    assert!(detail.l1_update().is_none());
}

#[tokio::test]
async fn get_block_by_slot_unknown_returns_none() {
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        OLBlockCommitment::new(0, null_blkid()),
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let detail = rpc.get_block_by_slot(42).await.expect("rpc call");
    assert!(detail.is_none());
}

// ── get_recent_blocks ──

#[tokio::test]
async fn get_recent_blocks_walks_backwards_in_order() {
    let block0 = make_block(0, 0, null_blkid());
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(1, 0, blkid0);
    let blkid1 = block1.header().compute_blkid();
    let block2 = make_block(2, 0, blkid1);
    let blkid2 = block2.header().compute_blkid();

    let tip = OLBlockCommitment::new(2, blkid2);
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block0, genesis_ol_state())
        .with_block_and_state(&block1, genesis_ol_state())
        .with_block_and_state(&block2, genesis_ol_state());
    let rpc = make_rpc(provider);

    let summaries = rpc.get_recent_blocks(3).await.expect("recent blocks");
    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].slot(), 0);
    assert_eq!(summaries[1].slot(), 1);
    assert_eq!(summaries[2].slot(), 2);
    assert_eq!(summaries[2].blkid(), blkid2);
    assert!(summaries.iter().all(|s| s.tx_count() == 0));
}

#[tokio::test]
async fn get_recent_blocks_zero_returns_empty() {
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        OLBlockCommitment::new(5, null_blkid()),
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let summaries = rpc.get_recent_blocks(0).await.expect("rpc call");
    assert!(summaries.is_empty());
}

#[tokio::test]
async fn get_recent_blocks_caps_at_genesis_when_count_exceeds_tip() {
    let block0 = make_block(0, 0, null_blkid());
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(1, 0, blkid0);
    let blkid1 = block1.header().compute_blkid();

    let tip = OLBlockCommitment::new(1, blkid1);
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block0, genesis_ol_state())
        .with_block_and_state(&block1, genesis_ol_state());
    let rpc = make_rpc(provider);

    let summaries = rpc.get_recent_blocks(10).await.expect("rpc call");
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].slot(), 0);
    assert_eq!(summaries[1].slot(), 1);
}

// ── get_block_transactions ──

#[tokio::test]
async fn get_block_transactions_empty_block_returns_empty() {
    let block = make_block(3, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(3, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, genesis_ol_state());
    let rpc = make_rpc(provider);

    let txs = rpc.get_block_transactions(3).await.expect("txs");
    assert!(txs.is_empty());
}

#[tokio::test]
async fn get_block_transactions_unknown_slot_errors() {
    let provider = MockProvider::new().with_sync_status(make_sync_status(
        OLBlockCommitment::new(0, null_blkid()),
        0,
        false,
        EpochCommitment::null(),
        EpochCommitment::null(),
        EpochCommitment::null(),
    ));
    let rpc = make_rpc(provider);

    let result = rpc.get_block_transactions(99).await;
    assert!(result.is_err());
}

// ── list_accounts ──

#[tokio::test]
async fn list_accounts_returns_ledger_entries() {
    let acct = AccountId::from([0x11; 32]);
    let block = make_block(4, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(4, blkid);

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(
            &block,
            ol_state_with_snark_account(acct, 4, 7, DEFAULT_NEXT_INBOX_MSG_IDX),
        );
    let rpc = make_rpc(provider);

    let entries = rpc
        .list_accounts(OLBlockOrTag::Slot(4))
        .await
        .expect("list accounts");
    let our_entry = entries
        .iter()
        .find(|e| e.id().0 == *acct.inner())
        .expect("account present in ledger");
    assert_eq!(our_entry.account_type(), RpcAccountType::Snark);
    let snark = our_entry.snark().expect("snark summary");
    assert_eq!(snark.seq_no(), 7);
}
