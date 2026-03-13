use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use strata_asm_common::AsmManifest;
use strata_db_types::DbResult;
use strata_identifiers::{
    AccountId, Buf32, Buf64, Epoch, Hash, L1BlockCommitment, L1BlockId, L1Height, OLBlockId, OLTxId,
};
use strata_ledger_types::{
    AccountTypeState, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_ol_chain_types_new::{
    OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
};
use strata_ol_mempool::{OLMempoolError, OLMempoolResult, OLMempoolTransaction};
use strata_ol_params::OLParams;
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer};
use strata_ol_rpc_types::{
    AccountExtraData, OLBlockOrTag, OLRpcProvider, RpcGenericAccountMessage, RpcOLTransaction,
    RpcSnarkAccountUpdate, RpcTransactionAttachment, RpcTransactionPayload,
};
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_primitives::{
    HexBytes, HexBytes32, OLBlockCommitment, epoch::EpochCommitment, prelude::BitcoinAmount,
};
use strata_snark_acct_types::Seqno;
use strata_status::OLSyncStatus;

use super::OLRpcServer;
use crate::rpc::errors::{
    INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, MEMPOOL_CAPACITY_ERROR_CODE, map_mempool_error_to_rpc,
};

// -- Mock provider --

type SubmitFn = Box<dyn Fn(OLMempoolTransaction) -> OLMempoolResult<OLTxId> + Send + Sync>;

struct MockProvider {
    blocks: HashMap<OLBlockId, OLBlock>,
    canonical_slots: HashMap<u64, OLBlockCommitment>,
    states: HashMap<OLBlockCommitment, Arc<OLState>>,
    epoch_commitments: HashMap<u64, EpochCommitment>,
    account_extra_data: HashMap<(AccountId, Epoch), AccountExtraData>,
    account_creation_epochs: HashMap<AccountId, Epoch>,
    manifests: HashMap<L1Height, AsmManifest>,
    sync_status: Option<OLSyncStatus>,
    submit_fn: SubmitFn,
}

impl MockProvider {
    fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            canonical_slots: HashMap::new(),
            states: HashMap::new(),
            epoch_commitments: HashMap::new(),
            account_extra_data: HashMap::new(),
            account_creation_epochs: HashMap::new(),
            manifests: HashMap::new(),
            sync_status: None,
            submit_fn: Box::new(|_| Ok(OLTxId::from(Buf32::from([0xAB; 32])))),
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

    fn with_epoch_commitment(mut self, epoch: u64, commitment: EpochCommitment) -> Self {
        self.epoch_commitments.insert(epoch, commitment);
        self
    }

    fn with_state_at(mut self, commitment: OLBlockCommitment, state: OLState) -> Self {
        self.states.insert(commitment, Arc::new(state));
        self
    }

    fn with_submit_fn(
        mut self,
        f: impl Fn(OLMempoolTransaction) -> OLMempoolResult<OLTxId> + Send + Sync + 'static,
    ) -> Self {
        self.submit_fn = Box::new(f);
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
        epoch: u64,
    ) -> DbResult<Option<EpochCommitment>> {
        Ok(self.epoch_commitments.get(&epoch).copied())
    }

    async fn get_account_extra_data(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<AccountExtraData>> {
        Ok(self.account_extra_data.get(&key).cloned())
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

    async fn submit_transaction(&self, tx: OLMempoolTransaction) -> OLMempoolResult<OLTxId> {
        (self.submit_fn)(tx)
    }
}

// -- Helpers --

fn test_account_id(byte: u8) -> AccountId {
    let mut bytes = [1u8; 32];
    bytes[0] = byte;
    AccountId::new(bytes)
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

fn genesis_ol_state() -> OLState {
    let params = OLParams::new_empty(test_l1_commitment());
    OLState::from_genesis_params(&params).expect("genesis state")
}

fn ol_state_with_snark_account(account_id: AccountId, seq_no: u64, slot: u64) -> OLState {
    let mut state = genesis_ol_state();
    state.set_cur_slot(slot);
    let snark = OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), Hash::zero());
    let new_acct = NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Snark(snark));
    state.create_new_account(account_id, new_acct).unwrap();
    state
        .update_account(account_id, |acct| {
            let s = acct.as_snark_account_mut().unwrap();
            s.set_proof_state_directly(Hash::zero(), 0, Seqno::from(seq_no));
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

fn make_rpc(provider: MockProvider) -> OLRpcServer<MockProvider> {
    OLRpcServer::new(provider)
}

fn make_gam_rpc_tx(target: AccountId, payload: Vec<u8>) -> RpcOLTransaction {
    let gam = RpcGenericAccountMessage::new(HexBytes32::from(*target.inner()), HexBytes(payload));
    RpcOLTransaction::new(
        RpcTransactionPayload::GenericAccountMessage(gam),
        RpcTransactionAttachment::new(None, None),
    )
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
        .with_block_and_state(&block0, ol_state_with_snark_account(account_id, 0, 0))
        .with_block_and_state(&block1, ol_state_with_snark_account(account_id, 1, 1))
        .with_block_and_state(&block2, ol_state_with_snark_account(account_id, 2, 2));
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

    let mut state = ol_state_with_snark_account(snark_id, 42, 0);
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
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 5, 20))
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
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 0, 5))
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

    let bad_tx = RpcOLTransaction::new(
        RpcTransactionPayload::SnarkAccountUpdate(RpcSnarkAccountUpdate::new(
            HexBytes32::from(*account_id.inner()),
            HexBytes(vec![0xDE, 0xAD]),
            HexBytes(vec![]),
        )),
        RpcTransactionAttachment::new(None, None),
    );

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
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 7, 5));
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
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 3, 10));
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
        .with_block_and_state(&block, ol_state_with_snark_account(account_id, 11, 8));
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
