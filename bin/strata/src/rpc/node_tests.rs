use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{ClientState, L1Status};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_identifiers::{
    AccountId, Buf32, Buf64, Hash, L1BlockCommitment, L1BlockId, OLBlockId, OLTxId,
};
use strata_ledger_types::{
    AccountTypeState, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_ol_chain_types_new::{
    OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
};
use strata_ol_mempool::{MempoolBuilder, OLMempoolConfig, OLMempoolError};
use strata_ol_params::OLParams;
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer};
use strata_ol_rpc_types::{
    OLBlockOrTag, RpcGenericAccountMessage, RpcOLTransaction, RpcSnarkAccountUpdate,
    RpcTransactionAttachment, RpcTransactionPayload,
};
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_primitives::{
    HexBytes, HexBytes32, OLBlockCommitment, epoch::EpochCommitment, prelude::BitcoinAmount,
};
use strata_snark_acct_types::Seqno;
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::{NodeStorage, create_node_storage};
use strata_tasks::TaskManager;
use threadpool::ThreadPool;
use tokio::runtime::Handle;

use super::OLRpcServer;
use crate::rpc::errors::{
    INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, MEMPOOL_CAPACITY_ERROR_CODE, map_mempool_error_to_rpc,
};

// -- Helpers --

fn create_test_storage() -> Arc<NodeStorage> {
    let pool = ThreadPool::new(1);
    let db = get_test_sled_backend();
    Arc::new(create_node_storage(db, pool).expect("create test storage"))
}

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

fn status_channel_no_ol() -> StatusChannel {
    StatusChannel::new(
        ClientState::new(None, None),
        test_l1_commitment(),
        L1Status::default(),
        None,
        None,
    )
}

fn status_channel_with_ol(
    tip: OLBlockCommitment,
    prev_epoch: EpochCommitment,
    finalized_epoch: EpochCommitment,
) -> StatusChannel {
    let ol_status = OLSyncStatus::new(tip, prev_epoch, finalized_epoch, test_l1_commitment());
    StatusChannel::new(
        ClientState::new(None, None),
        test_l1_commitment(),
        L1Status::default(),
        None,
        Some(OLSyncStatusUpdate::new(ol_status)),
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

async fn insert_epoch_summary(
    storage: &NodeStorage,
    epoch: u32,
    terminal: OLBlockCommitment,
    prev_terminal: OLBlockCommitment,
) {
    let summary = EpochSummary::new(
        epoch,
        terminal,
        prev_terminal,
        test_l1_commitment(),
        Buf32::zero(),
    );
    storage
        .ol_checkpoint()
        .insert_epoch_summary_async(summary)
        .await
        .expect("insert epoch summary");
}

async fn insert_block_with_state(storage: &NodeStorage, block: &OLBlock, state: OLState) {
    let blkid = block.header().compute_blkid();
    let slot = block.header().slot();
    storage
        .ol_block()
        .put_block_data_async(block.clone())
        .await
        .expect("insert block");
    storage
        .ol_state()
        .put_toplevel_ol_state_async(OLBlockCommitment::new(slot, blkid), state)
        .await
        .expect("insert state");
}

async fn launch_mempool(
    storage: Arc<NodeStorage>,
    status: &StatusChannel,
    tip: OLBlockCommitment,
) -> Arc<strata_ol_mempool::MempoolHandle> {
    let tm = TaskManager::new(Handle::current());
    let exec = tm.create_executor();
    let handle = MempoolBuilder::new(OLMempoolConfig::default(), storage, status.clone(), tip)
        .launch(&exec)
        .await
        .expect("launch mempool");
    Arc::new(handle)
}

/// Build an `OLRpcServer` backed by real storage with a genesis state at `tip`.
async fn setup_rpc(
    tip: OLBlockCommitment,
    prev_epoch: EpochCommitment,
    finalized: EpochCommitment,
) -> (OLRpcServer, Arc<NodeStorage>) {
    let storage = create_test_storage();
    let mut gs = genesis_ol_state();
    gs.set_cur_slot(tip.slot());
    storage
        .ol_state()
        .put_toplevel_ol_state_async(tip, gs)
        .await
        .expect("insert tip state");

    let status = status_channel_with_ol(tip, prev_epoch, finalized);
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage.clone(), Arc::new(status), mempool);
    (rpc, storage)
}

/// Build an `OLRpcServer` with empty accounts, so the mempool can validate txs.
async fn setup_rpc_with_accounts(tip: OLBlockCommitment, accounts: &[AccountId]) -> OLRpcServer {
    let storage = create_test_storage();
    let mut state = genesis_ol_state();
    state.set_cur_slot(tip.slot());
    for account_id in accounts {
        let new_acct = NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Empty);
        state.create_new_account(*account_id, new_acct).unwrap();
    }
    storage
        .ol_state()
        .put_toplevel_ol_state_async(tip, state)
        .await
        .expect("insert tip state");

    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    OLRpcServer::new(storage, Arc::new(status), mempool)
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
    let storage = create_test_storage();
    let tip = OLBlockCommitment::new(0, null_blkid());
    let mut gs = genesis_ol_state();
    gs.set_cur_slot(0);
    storage
        .ol_state()
        .put_toplevel_ol_state_async(tip, gs)
        .await
        .unwrap();

    let status = status_channel_no_ol();
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

    let result = rpc.chain_status().await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INTERNAL_ERROR_CODE);
}

#[tokio::test]
async fn chain_status_returns_correct_values() {
    let tip = OLBlockCommitment::new(100, OLBlockId::from(Buf32::from([1u8; 32])));
    let prev = EpochCommitment::new(1, 50, OLBlockId::from(Buf32::from([2u8; 32])));
    let finalized = EpochCommitment::new(0, 20, OLBlockId::from(Buf32::from([3u8; 32])));

    let (rpc, _storage) = setup_rpc(tip, prev, finalized).await;
    let status = rpc.chain_status().await.expect("chain_status");

    assert_eq!(status.latest().slot(), 100);
    assert_eq!(status.parent().epoch(), 1);
    assert_eq!(status.finalized().epoch(), 0);
    assert_eq!(status.finalized().last_slot(), 20);
}

// ── get_blocks_summaries ──

#[tokio::test]
async fn blocks_summaries_start_gt_end_returns_invalid_params() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let (rpc, _) = setup_rpc(tip, EpochCommitment::null(), EpochCommitment::null()).await;

    let result = rpc.get_blocks_summaries(test_account_id(1), 10, 5).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

#[tokio::test]
async fn blocks_summaries_no_block_at_end_returns_empty() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let (rpc, _) = setup_rpc(tip, EpochCommitment::null(), EpochCommitment::null()).await;

    let result = rpc
        .get_blocks_summaries(test_account_id(1), 0, 99)
        .await
        .expect("should succeed");
    assert!(result.is_empty());
}

#[tokio::test]
async fn blocks_summaries_returns_ascending_order() {
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    // Chain: block0 -> block1 -> block2
    let block0 = make_block(0, 0, null_blkid());
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(1, 0, blkid0);
    let blkid1 = block1.header().compute_blkid();
    let block2 = make_block(2, 0, blkid1);
    let blkid2 = block2.header().compute_blkid();

    insert_block_with_state(
        &storage,
        &block0,
        ol_state_with_snark_account(account_id, 0, 0),
    )
    .await;
    insert_block_with_state(
        &storage,
        &block1,
        ol_state_with_snark_account(account_id, 1, 1),
    )
    .await;
    insert_block_with_state(
        &storage,
        &block2,
        ol_state_with_snark_account(account_id, 2, 2),
    )
    .await;

    let tip = OLBlockCommitment::new(2, blkid2);
    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

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
    let storage = create_test_storage();
    let snark_id = test_account_id(1);
    let empty_id = test_account_id(2);

    let block = make_block(0, 0, null_blkid());
    let blkid = block.header().compute_blkid();

    // State with both account types
    let mut state = ol_state_with_snark_account(snark_id, 42, 0);
    let empty_acct = NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Empty);
    state.create_new_account(empty_id, empty_acct).unwrap();
    insert_block_with_state(&storage, &block, state).await;

    let tip = OLBlockCommitment::new(0, blkid);
    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

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
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let (rpc, _) = setup_rpc(tip, EpochCommitment::null(), EpochCommitment::null()).await;

    let result = rpc.get_acct_epoch_summary(test_account_id(1), 99).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn epoch_summary_nonexistent_account_errors() {
    let storage = create_test_storage();

    let block = make_block(10, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(10, blkid);

    insert_block_with_state(&storage, &block, genesis_ol_state()).await;
    insert_epoch_summary(
        &storage,
        0,
        terminal,
        OLBlockCommitment::new(0, null_blkid()),
    )
    .await;

    let status = status_channel_with_ol(terminal, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, terminal).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

    let result = rpc.get_acct_epoch_summary(test_account_id(99), 0).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

#[tokio::test]
async fn epoch_summary_valid_snark_account() {
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    let block = make_block(20, 1, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(20, blkid);

    insert_block_with_state(
        &storage,
        &block,
        ol_state_with_snark_account(account_id, 5, 20),
    )
    .await;

    // Epoch 1 summary
    let prev_terminal = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    insert_epoch_summary(&storage, 1, terminal, prev_terminal).await;
    // Epoch 0 summary (needed for prev lookup)
    insert_epoch_summary(
        &storage,
        0,
        prev_terminal,
        OLBlockCommitment::new(0, null_blkid()),
    )
    .await;

    let status = status_channel_with_ol(
        terminal,
        EpochCommitment::new(1, 20, blkid),
        EpochCommitment::null(),
    );
    let mempool = launch_mempool(storage.clone(), &status, terminal).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

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
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(5, blkid);

    insert_block_with_state(
        &storage,
        &block,
        ol_state_with_snark_account(account_id, 0, 5),
    )
    .await;
    insert_epoch_summary(
        &storage,
        0,
        terminal,
        OLBlockCommitment::new(0, null_blkid()),
    )
    .await;

    let status = status_channel_with_ol(terminal, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, terminal).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

    let summary = rpc
        .get_acct_epoch_summary(account_id, 0)
        .await
        .expect("epoch 0");
    assert_eq!(summary.prev_epoch_commitment().epoch(), 0);
    assert_eq!(summary.prev_epoch_commitment().last_slot(), 0);
}

#[tokio::test]
async fn epoch_summary_non_snark_account() {
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let terminal = OLBlockCommitment::new(5, blkid);

    insert_block_with_state(&storage, &block, ol_state_with_empty_account(account_id, 5)).await;
    insert_epoch_summary(
        &storage,
        0,
        terminal,
        OLBlockCommitment::new(0, null_blkid()),
    )
    .await;

    let status = status_channel_with_ol(terminal, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, terminal).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

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
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));

    let rpc = setup_rpc_with_accounts(tip, &[account_id]).await;

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
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));

    let rpc = setup_rpc_with_accounts(tip, &[account_id]).await;

    // Garbage bytes that can't be decoded as UpdateOperationData SSZ
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
    let existing = test_account_id(1);
    let missing = test_account_id(99);
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));

    let rpc = setup_rpc_with_accounts(tip, &[existing]).await;

    let tx = make_gam_rpc_tx(missing, vec![1, 2, 3]);
    let result = rpc.submit_transaction(tx).await;
    assert!(result.is_err());
    // AccountDoesNotExist maps to INVALID_PARAMS_CODE via map_mempool_error_to_rpc
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

// ── get_snark_account_state ──

#[tokio::test]
async fn snark_account_state_latest_returns_state() {
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(5, blkid);

    insert_block_with_state(
        &storage,
        &block,
        ol_state_with_snark_account(account_id, 7, 5),
    )
    .await;

    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

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
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    let block = make_block(10, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(10, blkid);

    insert_block_with_state(
        &storage,
        &block,
        ol_state_with_snark_account(account_id, 3, 10),
    )
    .await;

    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

    let state = rpc
        .get_snark_account_state(account_id, OLBlockOrTag::Slot(10))
        .await
        .expect("snark state")
        .expect("should be Some");

    assert_eq!(state.seq_no(), 3);
}

#[tokio::test]
async fn snark_account_state_non_snark_returns_none() {
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    let block = make_block(5, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(5, blkid);

    insert_block_with_state(&storage, &block, ol_state_with_empty_account(account_id, 5)).await;

    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

    let result = rpc
        .get_snark_account_state(account_id, OLBlockOrTag::Latest)
        .await
        .expect("should succeed");

    assert!(result.is_none());
}

#[tokio::test]
async fn snark_account_state_missing_account_returns_none() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let (rpc, _) = setup_rpc(tip, EpochCommitment::null(), EpochCommitment::null()).await;

    let result = rpc
        .get_snark_account_state(test_account_id(99), OLBlockOrTag::Latest)
        .await
        .expect("should succeed");

    assert!(result.is_none());
}

#[tokio::test]
async fn snark_account_state_no_ol_sync_returns_error() {
    let storage = create_test_storage();
    let tip = OLBlockCommitment::new(0, null_blkid());
    let mut gs = genesis_ol_state();
    gs.set_cur_slot(0);
    storage
        .ol_state()
        .put_toplevel_ol_state_async(tip, gs)
        .await
        .unwrap();

    let status = status_channel_no_ol();
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

    let result = rpc
        .get_snark_account_state(test_account_id(1), OLBlockOrTag::Latest)
        .await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INTERNAL_ERROR_CODE);
}

#[tokio::test]
async fn snark_account_state_by_block_id() {
    let storage = create_test_storage();
    let account_id = test_account_id(1);

    let block = make_block(8, 0, null_blkid());
    let blkid = block.header().compute_blkid();
    let tip = OLBlockCommitment::new(8, blkid);

    insert_block_with_state(
        &storage,
        &block,
        ol_state_with_snark_account(account_id, 11, 8),
    )
    .await;

    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

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
    let storage = create_test_storage();

    let block0 = make_block(0, 0, null_blkid());
    let blkid0 = block0.header().compute_blkid();
    let block1 = make_block(1, 0, blkid0);
    let blkid1 = block1.header().compute_blkid();
    let block2 = make_block(2, 0, blkid1);
    let blkid2 = block2.header().compute_blkid();

    insert_block_with_state(&storage, &block0, genesis_ol_state()).await;
    insert_block_with_state(&storage, &block1, genesis_ol_state()).await;
    insert_block_with_state(&storage, &block2, genesis_ol_state()).await;

    let tip = OLBlockCommitment::new(2, blkid2);
    let status = status_channel_with_ol(tip, EpochCommitment::null(), EpochCommitment::null());
    let mempool = launch_mempool(storage.clone(), &status, tip).await;
    let rpc = OLRpcServer::new(storage, Arc::new(status), mempool);

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
    let (rpc, _) = setup_rpc(tip, EpochCommitment::null(), EpochCommitment::null()).await;

    let result = rpc.get_raw_blocks_range(10, 5).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}

#[tokio::test]
async fn raw_blocks_range_exceeds_max_returns_invalid_params() {
    let tip = OLBlockCommitment::new(10, OLBlockId::from(Buf32::from([1u8; 32])));
    let (rpc, _) = setup_rpc(tip, EpochCommitment::null(), EpochCommitment::null()).await;

    // MAX_RAW_BLOCKS_RANGE is 5000, request 5001
    let result = rpc.get_raw_blocks_range(0, 5000).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), INVALID_PARAMS_CODE);
}
