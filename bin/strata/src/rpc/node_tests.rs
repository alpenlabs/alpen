use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use proptest::prelude::*;
use strata_acct_types::{MessageEntry, MsgPayload, TxEffects};
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::{DbError, DbResult, types::AccountExtraDataEntry};
use strata_identifiers::*;
use strata_ledger_types::*;
use strata_ol_chain_types_new::{test_utils::transaction_payload_strategy, *};
use strata_ol_mempool::{OLMempoolError, OLMempoolResult};
use strata_ol_params::OLParams;
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer};
use strata_ol_rpc_types::*;
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_primitives::{
    HexBytes, HexBytes32, OLBlockCommitment, epoch::EpochCommitment, prelude::BitcoinAmount,
};
use strata_snark_acct_types::{ProofState, Seqno, UpdateInputData, UpdateStateData};
use strata_status::OLSyncStatus;
use tokio::runtime::Builder;

use super::{
    OLRpcServer, extract_account_activity_from_block, resolve_update_extra_data_for_block,
};
use crate::rpc::errors::{
    INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, MEMPOOL_CAPACITY_ERROR_CODE, map_mempool_error_to_rpc,
};

// -- Mock provider --

type SubmitFn = Box<dyn Fn(OLTransaction) -> OLMempoolResult<OLTxId> + Send + Sync>;
type InboxFetchFn = Box<dyn Fn(AccountId, u64, u64) -> DbResult<Vec<MessageEntry>> + Send + Sync>;

struct MockProvider {
    blocks: HashMap<OLBlockId, OLBlock>,
    canonical_slots: HashMap<u64, OLBlockCommitment>,
    states: HashMap<OLBlockCommitment, Arc<OLState>>,
    epoch_commitments: HashMap<Epoch, EpochCommitment>,
    epoch_summaries: HashMap<EpochCommitment, EpochSummary>,
    checkpoint_l1_refs: HashMap<EpochCommitment, CheckpointL1Ref>,
    account_extra_data: HashMap<(AccountId, Epoch), AccountExtraData>,
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
            account_extra_data: HashMap::new(),
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
        extra_data: Vec<u8>,
        block: OLBlockCommitment,
    ) -> Self {
        let entry = AccountExtraDataEntry::new(extra_data, block);
        let key = (account_id, epoch);
        if let Some(entries) = self.account_extra_data.get_mut(&key) {
            entries.push(entry);
        } else {
            self.account_extra_data
                .insert(key, AccountExtraData::new(entry));
        }
        self
    }

    fn with_account_extra_data_at_terminal(
        self,
        account_id: AccountId,
        epoch: Epoch,
        extra_data: Vec<u8>,
        commitment: EpochCommitment,
    ) -> Self {
        self.with_account_extra_data(
            account_id,
            epoch,
            extra_data,
            commitment.to_block_commitment(),
        )
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

    async fn get_account_extra_data(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<AccountExtraData>> {
        Ok(self.account_extra_data.get(&key).cloned())
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

fn make_block(slot: u64, epoch: u32, parent: OLBlockId) -> OLBlock {
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

fn make_block_with_txs(
    slot: u64,
    epoch: Epoch,
    parent: OLBlockId,
    txs: Vec<OLTransaction>,
) -> OLBlock {
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
    let tx_segment = OLTxSegment::new(txs).expect("tx segment");
    let body = OLBlockBody::new_common(tx_segment);
    OLBlock::new(signed, body)
}

fn genesis_ol_state() -> OLState {
    let params = OLParams::new_empty(test_l1_commitment());
    OLState::from_genesis_params(&params).expect("genesis state")
}

fn ol_state_with_snark_account(
    account_id: AccountId,
    slot: u64,
    seq_no: u64,
    next_inbox_msg_idx: u64,
) -> OLState {
    let mut state = genesis_ol_state();
    state.set_cur_slot(slot);
    let snark = OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), Hash::zero());
    let new_acct = NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Snark(snark));
    state.create_new_account(account_id, new_acct).unwrap();
    state
        .update_account(account_id, |acct| {
            let s = acct.as_snark_account_mut().unwrap();
            s.set_proof_state_directly(Hash::zero(), next_inbox_msg_idx, Seqno::from(seq_no));
        })
        .unwrap();
    state
}

fn ol_state_with_empty_account(account_id: AccountId, slot: u64) -> OLState {
    let mut state = genesis_ol_state();
    state.set_cur_slot(slot);
    let new_acct = NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Empty);
    state.create_new_account(account_id, new_acct).unwrap();
    state
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

fn make_sau_tx(
    target: AccountId,
    seq_no: u64,
    next_inbox_msg_idx: u64,
    inner_state_tag: u8,
    extra_data: Vec<u8>,
    processed_messages: Vec<MessageEntry>,
    effects: TxEffects,
) -> OLTransaction {
    let proof_state = SauTxProofState::new(next_inbox_msg_idx, fixed_buf32(inner_state_tag));
    let update_data = SauTxUpdateData::new(seq_no, proof_state, extra_data);
    let operation_data = SauTxOperationData::new(
        update_data,
        processed_messages,
        SauTxLedgerRefs::new_empty(),
    );
    let payload = TransactionPayload::SnarkAccountUpdate(SauTxPayload::new(target, operation_data));
    let tx_data = OLTransactionData::new(payload, effects);
    OLTransaction::new(tx_data, TxProofs::new_empty())
}

/// Retargets a generated payload to the given account.
fn retarget_payload(payload: TransactionPayload, target: AccountId) -> TransactionPayload {
    match payload {
        TransactionPayload::GenericAccountMessage(_) => {
            TransactionPayload::GenericAccountMessage(GamTxPayload::new(target).expect("gam"))
        }
        TransactionPayload::SnarkAccountUpdate(sau) => TransactionPayload::SnarkAccountUpdate(
            SauTxPayload::new(target, sau.operation().clone()),
        ),
    }
}

/// Generates an arbitrary OL transaction targeting either `account_id` or `other_account`,
/// with a single effect message aimed at one of the two. Uses chain-types strategies for
/// payload/constraint generation.
fn arbitrary_tx_strategy(
    account_id: AccountId,
    other_account: AccountId,
) -> impl Strategy<Value = OLTransaction> {
    (
        transaction_payload_strategy(),
        any::<bool>(),
        any::<bool>(),
        any::<u64>(),
        prop::collection::vec(any::<u8>(), 0..8),
    )
        .prop_map(
            move |(payload, to_target, effect_to_target, effect_val, effect_data)| {
                let target = if to_target { account_id } else { other_account };
                let payload = retarget_payload(payload, target);
                let effect_dest = if effect_to_target {
                    account_id
                } else {
                    other_account
                };
                let mut effects = TxEffects::default();
                effects.push_message(effect_dest, effect_val, effect_data);
                let data = OLTransactionData::new(payload, effects);
                OLTransaction::new(data, TxProofs::new_empty())
            },
        )
}

/// Returns the sender that `extract_account_activity_from_block` would derive for a tx.
fn expected_sender(tx: &OLTransaction) -> AccountId {
    match tx.payload() {
        TransactionPayload::GenericAccountMessage(_) => strata_ol_stf::SEQUENCER_ACCT_ID,
        TransactionPayload::SnarkAccountUpdate(p) => *p.target(),
    }
}

fn test_epoch_commitment(epoch: Epoch, slot: u64, blkid_tag: u8) -> EpochCommitment {
    EpochCommitment::new(epoch, slot, fixed_ol_block_id(blkid_tag))
}

fn make_message_entry(
    source: AccountId,
    incl_epoch: Epoch,
    payload_value_sat: u64,
    payload_buf: Vec<u8>,
) -> MessageEntry {
    let payload = MsgPayload::new(BitcoinAmount::from_sat(payload_value_sat), payload_buf);
    MessageEntry::new(source, incl_epoch, payload)
}

fn rpc_messages_to_entries(messages: &[RpcMessageEntry]) -> Vec<MessageEntry> {
    messages.iter().cloned().map(Into::into).collect()
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
        fixed_buf32(0xAA),
        fixed_buf32(0xBB),
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
        checkpoint_txid,
        checkpoint_wtxid,
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
            assert_eq!(l1_reference.txid, checkpoint_txid);
            assert_eq!(l1_reference.wtxid, checkpoint_wtxid);
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
        checkpoint_txid,
        checkpoint_wtxid,
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
            assert_eq!(l1_reference.txid, checkpoint_txid);
            assert_eq!(l1_reference.wtxid, checkpoint_wtxid);
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
        checkpoint_txid,
        checkpoint_wtxid,
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

    let block0 = make_block(0, 0, null_blkid());
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(1, 0, blkid0);
    let blkid1 = block1.header().compute_blkid();
    let block2 = make_block(2, 0, blkid1);
    let blkid2 = block2.header().compute_blkid();

    let tip = OLBlockCommitment::new(2, blkid2);
    let prev = EpochCommitment::new(1, 50, OLBlockId::from(Buf32::from([2u8; 32])));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            tip,
            1,
            false,
            prev,
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(
            &block0,
            ol_state_with_snark_account(account_id, 0, 0, DEFAULT_NEXT_INBOX_MSG_IDX),
        )
        .with_block_and_state(
            &block1,
            ol_state_with_snark_account(account_id, 1, 1, DEFAULT_NEXT_INBOX_MSG_IDX),
        )
        .with_block_and_state(
            &block2,
            ol_state_with_snark_account(account_id, 2, 2, DEFAULT_NEXT_INBOX_MSG_IDX),
        );
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 2)
        .await
        .expect("summaries");

    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].block_commitment().slot(), 0);
    assert_eq!(summaries[1].block_commitment().slot(), 1);
    assert_eq!(summaries[2].block_commitment().slot(), 2);
}

#[tokio::test]
async fn blocks_summaries_snark_vs_non_snark() {
    let snark_id = test_account_id(1);
    let empty_id = test_account_id(2);

    let block = make_block(0, 0, null_blkid());
    let blkid = block.header().compute_blkid();

    let mut state = ol_state_with_snark_account(snark_id, 0, 42, DEFAULT_NEXT_INBOX_MSG_IDX);
    let empty_acct = NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Empty);
    state.create_new_account(empty_id, empty_acct).unwrap();

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

// ── get_blocks_summaries: activity extraction edge cases ──

#[tokio::test]
async fn blocks_summaries_empty_tx_segment_produces_no_activity() {
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

#[test]
fn extract_activity_none_tx_segment() {
    let account_id = test_account_id(1);
    let state = ol_state_with_snark_account(account_id, 0, 5, 3);
    let account_state = state.get_account_state(account_id).unwrap().unwrap();

    let (updates, inbox) = extract_account_activity_from_block(account_id, 0, None, account_state);
    assert!(updates.is_empty());
    assert!(inbox.is_empty());
}

// ── get_blocks_summaries: extra data resolution edge cases ──

#[tokio::test]
async fn blocks_summaries_extra_data_uses_tx_payload_when_no_index() {
    let account_id = test_account_id(1);
    let tx = make_sau_tx(
        account_id,
        3,
        1,
        0x63,
        vec![0xDE, 0xAD],
        Vec::new(),
        TxEffects::default(),
    );
    let block = make_block_with_txs(0, 0, null_blkid(), vec![tx.clone()]);
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
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 4, 1));
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("summaries");
    let update: UpdateInputData = summaries[0].updates()[0].clone().into();
    let expected = match tx.payload() {
        TransactionPayload::SnarkAccountUpdate(p) => p.operation().update().extra_data(),
        _ => unreachable!(),
    };
    assert_eq!(update.extra_data(), expected);
}

#[tokio::test]
async fn blocks_summaries_extra_data_prefers_index_over_tx_payload() {
    let account_id = test_account_id(1);
    let indexed = vec![0xF0, 0x0D];
    let tx = make_sau_tx(
        account_id,
        6,
        2,
        0x66,
        vec![0x01, 0x02],
        Vec::new(),
        TxEffects::default(),
    );
    let block = make_block_with_txs(0, 0, null_blkid(), vec![tx]);
    let commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
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
        .with_account_extra_data(account_id, 0, indexed.clone(), commitment);
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("summaries");
    let update: UpdateInputData = summaries[0].updates()[0].clone().into();
    assert_eq!(update.extra_data(), indexed.as_slice());
}

#[tokio::test]
async fn blocks_summaries_extra_data_falls_back_for_non_matching_block() {
    let account_id = test_account_id(1);
    let tx = make_sau_tx(
        account_id,
        9,
        2,
        0x90,
        vec![0xCA, 0xFE],
        Vec::new(),
        TxEffects::default(),
    );
    let block = make_block_with_txs(0, 0, null_blkid(), vec![tx.clone()]);
    let commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
    let wrong_block = OLBlockCommitment::new(99, fixed_ol_block_id(0x99));
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 10, 2))
        .with_account_extra_data(account_id, 0, vec![0xF0, 0x0D], wrong_block);
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("summaries");
    let update: UpdateInputData = summaries[0].updates()[0].clone().into();
    let expected = match tx.payload() {
        TransactionPayload::SnarkAccountUpdate(p) => p.operation().update().extra_data(),
        _ => unreachable!(),
    };
    assert_eq!(update.extra_data(), expected);
}

#[tokio::test]
async fn blocks_summaries_extra_data_partial_mismatch_applies_available_entries() {
    let account_id = test_account_id(1);
    let tx1 = make_sau_tx(
        account_id,
        1,
        1,
        0xA1,
        vec![0x01],
        Vec::new(),
        TxEffects::default(),
    );
    let tx2 = make_sau_tx(
        account_id,
        2,
        2,
        0xA2,
        vec![0x02],
        Vec::new(),
        TxEffects::default(),
    );
    let block = make_block_with_txs(0, 0, null_blkid(), vec![tx1, tx2]);
    let commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            commitment,
            0,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 3, 2))
        .with_account_extra_data(account_id, 0, vec![0xFE], commitment);
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 0)
        .await
        .expect("summaries");
    let u0: UpdateInputData = summaries[0].updates()[0].clone().into();
    let u1: UpdateInputData = summaries[0].updates()[1].clone().into();
    assert_eq!(u0.extra_data(), [0xFE].as_slice());
    assert_eq!(u1.extra_data(), [0x02].as_slice());
}

#[tokio::test]
async fn blocks_summaries_extra_data_cache_across_epoch_blocks() {
    let account_id = test_account_id(1);
    let epoch: Epoch = 7;
    let b0_tx = make_sau_tx(
        account_id,
        21,
        1,
        0x21,
        vec![0x11],
        Vec::new(),
        TxEffects::default(),
    );
    let b0 = make_block_with_txs(0, epoch, null_blkid(), vec![b0_tx]);
    let b0c = OLBlockCommitment::new(0, b0.header().compute_blkid());

    let b1_tx = make_sau_tx(
        account_id,
        22,
        2,
        0x22,
        vec![0x22],
        Vec::new(),
        TxEffects::default(),
    );
    let b1 = make_block_with_txs(1, epoch, b0.header().compute_blkid(), vec![b1_tx]);
    let b1c = OLBlockCommitment::new(1, b1.header().compute_blkid());

    let provider = MockProvider::new()
        .with_sync_status(make_sync_status(
            b1c,
            epoch,
            false,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
        ))
        .with_block_and_state(&b0, ol_state_with_snark_account(account_id, 0, 21, 1))
        .with_block_and_state(&b1, ol_state_with_snark_account(account_id, 1, 22, 2))
        .with_account_extra_data(account_id, epoch, vec![0xA0], b0c)
        .with_account_extra_data(account_id, epoch, vec![0xA1], b1c);
    let rpc = make_rpc(provider);

    let summaries = rpc
        .get_blocks_summaries(account_id, 0, 1)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 2);
    let u0: UpdateInputData = summaries[0].updates()[0].clone().into();
    let u1: UpdateInputData = summaries[1].updates()[0].clone().into();
    assert_eq!(u0.extra_data(), [0xA0].as_slice());
    assert_eq!(u1.extra_data(), [0xA1].as_slice());
}

// ── get_blocks_summaries: property tests ──

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]

    /// Update count matches SAU txs targeting this account. Each update reflects its
    /// source SAU fields. Inbox messages match effect messages destined for this account
    /// with correct sender derivation (sequencer for GAM, target for SAU).
    #[test]
    fn blocks_summaries_reflects_block_txs_for_snark_account(
        txs in prop::collection::vec(
            arbitrary_tx_strategy(test_account_id(1), test_account_id(2)),
            0..12
        )
    ) {
        let account_id = test_account_id(1);
        let block_epoch: Epoch = 5;
        let block = make_block_with_txs(0, block_epoch, null_blkid(), txs.clone());
        let commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
        let provider = MockProvider::new()
            .with_sync_status(make_sync_status(commitment, block_epoch, false, EpochCommitment::null(), EpochCommitment::null(), EpochCommitment::null()))
            .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 99, 99));
        let rpc = make_rpc(provider);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        let summaries = rt
            .block_on(async { rpc.get_blocks_summaries(account_id, 0, 0).await })
            .expect("summaries");
        prop_assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];

        // Collect SAU txs targeting this account from the input.
        let target_saus: Vec<&SauTxPayload> = txs.iter().filter_map(|tx| match tx.payload() {
            TransactionPayload::SnarkAccountUpdate(p) if *p.target() == account_id => Some(p),
            _ => None,
        }).collect();

        // Update count matches SAU count targeting this account.
        prop_assert_eq!(summary.updates().len(), target_saus.len());

        // Each update reflects its source SAU fields.
        for (rpc_update, sau) in summary.updates().iter().zip(target_saus.iter()) {
            let update: UpdateInputData = rpc_update.clone().into();
            let op = sau.operation();
            prop_assert_eq!(update.seq_no(), op.update().seq_no());
            prop_assert_eq!(update.extra_data(), op.update().extra_data());
            prop_assert_eq!(
                update.new_state().inner_state(),
                op.update().proof_state().inner_state_root()
            );
            prop_assert_eq!(
                update.new_state().next_inbox_msg_idx(),
                op.update().proof_state().new_next_msg_idx()
            );
            let expected_msgs: Vec<_> = op.messages_iter().cloned().collect();
            prop_assert_eq!(update.processed_messages(), expected_msgs.as_slice());
        }

        // Inbox messages match effect messages destined for this account.
        let expected_inbox: Vec<_> = txs.iter()
            .flat_map(|tx| {
                let sender = expected_sender(tx);
                tx.data().effects().messages_iter()
                    .filter(|msg| msg.dest() == account_id)
                    .map(move |msg| (sender, msg.payload().clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let actual_inbox = rpc_messages_to_entries(summary.new_inbox_messages());
        prop_assert_eq!(actual_inbox.len(), expected_inbox.len());

        for (actual, (exp_sender, exp_payload)) in actual_inbox.iter().zip(expected_inbox.iter()) {
            prop_assert_eq!(actual.source, *exp_sender);
            prop_assert_eq!(actual.incl_epoch(), block_epoch);
            prop_assert_eq!(&actual.payload, exp_payload);
        }
    }

    /// Non-snark accounts never collect inbox messages, regardless of what effect
    /// messages target them. SAU updates targeting the account are still reported.
    #[test]
    fn blocks_summaries_non_snark_never_collects_inbox_but_reports_updates(
        txs in prop::collection::vec(
            arbitrary_tx_strategy(test_account_id(1), test_account_id(2)),
            0..12
        )
    ) {
        let account_id = test_account_id(1);
        let block_epoch: Epoch = 12;
        let block = make_block_with_txs(0, block_epoch, null_blkid(), txs.clone());
        let commitment = OLBlockCommitment::new(0, block.header().compute_blkid());
        let provider = MockProvider::new()
            .with_sync_status(make_sync_status(commitment, block_epoch, false, EpochCommitment::null(), EpochCommitment::null(), EpochCommitment::null()))
            .with_block_and_state(&block, ol_state_with_empty_account(account_id, 0));
        let rpc = make_rpc(provider);

        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        let summaries = rt
            .block_on(async { rpc.get_blocks_summaries(account_id, 0, 0).await })
            .expect("summaries");

        prop_assert_eq!(summaries.len(), 1);
        prop_assert!(summaries[0].new_inbox_messages().is_empty());

        // SAU updates targeting this account are still collected even for non-snark.
        let expected_update_count = txs.iter().filter(|tx| matches!(
            tx.payload(),
            TransactionPayload::SnarkAccountUpdate(p) if *p.target() == account_id
        )).count();
        prop_assert_eq!(summaries[0].updates().len(), expected_update_count);
    }

    /// When index entries match the block commitment, `resolve_update_extra_data_for_block`
    /// overwrites each update's extra_data with the index value. When entries don't match,
    /// the original tx payload extra_data is preserved.
    #[test]
    fn extra_data_resolved_from_matching_block(
        sau_payloads in prop::collection::vec(
            transaction_payload_strategy().prop_filter_map("SAU only", |p| match p {
                TransactionPayload::SnarkAccountUpdate(sau) => Some(sau),
                _ => None,
            }),
            1..6
        ),
        indexed_extra_datas in prop::collection::vec(
            prop::collection::vec(any::<u8>(), 1..16),
            0..8
        ),
        matching in any::<bool>(),
    ) {
        let commitment = OLBlockCommitment::new(0, fixed_ol_block_id(1));
        let other_commitment = OLBlockCommitment::new(99, fixed_ol_block_id(99));

        // Build updates from the generated SAU payloads (same way production code does).
        let mut updates: Vec<UpdateInputData> = sau_payloads.iter().map(|sau| {
            let op = sau.operation();
            UpdateInputData::new(
                op.update().seq_no(),
                op.messages_iter().cloned().collect(),
                UpdateStateData::new(
                    ProofState::new(
                        op.update().proof_state().inner_state_root(),
                        op.update().proof_state().new_next_msg_idx(),
                    ),
                    op.update().extra_data().to_vec(),
                ),
            )
        }).collect();
        let original_extra_datas: Vec<Vec<u8>> = updates.iter()
            .map(|u| u.extra_data().to_vec())
            .collect();

        // Build index entries — either matching or non-matching block commitment.
        let entry_commitment = if matching { commitment } else { other_commitment };
        let entries: Option<AccountExtraData> = if indexed_extra_datas.is_empty() {
            None
        } else {
            let mut entries = AccountExtraData::new(
                AccountExtraDataEntry::new(indexed_extra_datas[0].clone(), entry_commitment),
            );
            for ed in &indexed_extra_datas[1..] {
                entries.push(AccountExtraDataEntry::new(ed.clone(), entry_commitment));
            }
            Some(entries)
        };

        resolve_update_extra_data_for_block(
            test_account_id(1), 0, commitment, &mut updates, entries.as_ref(),
        );

        for (i, update) in updates.iter().enumerate() {
            if matching && i < indexed_extra_datas.len() {
                // Index was applied.
                prop_assert_eq!(update.extra_data(), indexed_extra_datas[i].as_slice());
            } else {
                // Original tx payload preserved.
                prop_assert_eq!(update.extra_data(), original_extra_datas[i].as_slice());
            }
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
    assert!(summary.update_input().is_none());
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
        .with_account_extra_data_at_terminal(account_id, epoch, vec![2, 2, 5], epoch_commitment)
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
    let update = summary.update_input().expect("update input");
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
        .with_account_extra_data_at_terminal(account_id, epoch, vec![0, 3], epoch_commitment)
        .with_inbox_fetch_fn(inbox_fetch_panic("epoch 0 should not fetch inbox messages"));
    let rpc = make_rpc(provider);

    let summary = rpc
        .get_acct_epoch_summary(account_id, epoch)
        .await
        .expect("epoch summary");
    let update = summary.update_input().expect("update input");
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
        .with_account_extra_data_at_terminal(account_id, epoch, vec![4, 7], epoch_commitment)
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
    let update = summary.update_input().expect("update input");
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
        .with_account_extra_data_at_terminal(account_id, epoch, vec![3], epoch_commitment)
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
    let update = summary.update_input().expect("update input");
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
    assert!(summary.update_input().is_none());
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
        .with_account_extra_data_at_terminal(account_id, epoch, vec![1, 0x10], epoch_commitment)
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
