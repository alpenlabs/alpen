//! Signed block admission through the production FCM adapter, worker and sled managers.
//!
//! L1/ASM outputs are seeded fixtures; no Bitcoin RPC, ASM execution or proof guest runs here.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::Result;
use bitcoin::Network;
use bitcoind_async_client::{Auth, Client};
use strata_acct_types::{AccountSerial, BitcoinAmount};
use strata_asm_common::AsmManifest;
use strata_asm_params::AsmParams;
use strata_btc_verification::L1Anchor;
use strata_chain_worker::{
    ChainWorkerHandle, ManifestPendingReason, WorkerError, start_chain_worker_service_from_ctx,
};
use strata_config::Config;
use strata_consensus_logic::{
    ExecutionDeferral, FcmServiceHandle, message::ForkChoiceMessage, start_fcm_service,
};
use strata_csm_types::{ClientState, L1Status};
use strata_csm_worker::CsmWorkerStatus;
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_types::{DbError, MmrId, ol_block::BlockStatus};
use strata_identifiers::{AccountId, Buf32, L1BlockCommitment, L1BlockId, OLBlockId, SubjectId};
use strata_l1_txfmt::MagicBytes;
use strata_node_context::NodeContext;
use strata_ol_chain_types_v1::{
    OLBlockV1, SignedOLBlockHeaderV1,
    test_utils::{schnorr_predicate, test_schnorr_keypair},
    verify_sequencer_predicate_signature,
};
use strata_ol_genesis::{GenesisArtifacts, build_genesis_artifacts};
use strata_ol_params::{GenesisSnarkAccountData, OLParams, OLRuntimeParams};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IAccountState, IStateAccessor};
use strata_ol_stf_v1::{
    BlockComponents, BlockInfo, ExecError,
    test_utils::{execute_block, make_deposit_manifest_for_account, make_empty_manifest},
};
use strata_predicate::PredicateKey;
use strata_primitives::{OLBlockCommitment, crypto::sign_schnorr_sig};
use strata_service::{
    Response, Service, ServiceBuilder, ServiceMonitor, ServiceState, SyncService,
};
use strata_status::StatusChannel;
use strata_storage::{NodeStorage, create_node_storage};
use tokio::{
    pin,
    runtime::Handle,
    time::{sleep, timeout},
};

use super::{StrataFcmContext, WorkerFailureOutcome, classify_worker_failure};

/// Supplies only CSM's finality status; admission and execution use production services.
struct FixedCsmStatus;

impl ServiceState for FixedCsmStatus {
    fn name(&self) -> &str {
        "test_csm_status"
    }

    fn span_prefix(&self) -> &str {
        "test_csm_status"
    }
}

impl Service for FixedCsmStatus {
    type State = Self;
    type Msg = ();
    type Status = CsmWorkerStatus;

    fn get_status(_: &Self) -> CsmWorkerStatus {
        CsmWorkerStatus {
            cur_block: None,
            last_processed_epoch: None,
            last_confirmed_epoch: None,
            last_finalized_epoch: None,
        }
    }
}

impl SyncService for FixedCsmStatus {
    fn process_input(_: &mut Self, _: ()) -> Result<Response> {
        Ok(Response::Continue)
    }
}

struct Harness {
    node: NodeContext,
    ctx: Arc<StrataFcmContext>,
    worker: Arc<ChainWorkerHandle>,
    genesis: GenesisArtifacts,
    account: AccountId,
    serial: AccountSerial,
    secret: Buf32,
    predicate: PredicateKey,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.task_manager().get_shutdown_signal().send();
    }
}

impl Harness {
    async fn new() -> Result<Self> {
        let account = AccountId::from([17; 32]);
        let params = Arc::new(
            OLParams::builder(OLRuntimeParams::test_default())
                .genesis_accounts(BTreeMap::from([(
                    account,
                    GenesisSnarkAccountData {
                        predicate: PredicateKey::always_accept(),
                        inner_state: Buf32::zero(),
                        balance: BitcoinAmount::default(),
                    },
                )]))
                .build(),
        );
        let genesis = build_genesis_artifacts(&params)?;
        let serial = genesis
            .ol_state
            .get_account_state(&account)
            .unwrap()
            .serial();

        let storage = Arc::new(create_node_storage(
            get_test_sled_backend(),
            Handle::current(),
        )?);
        storage
            .ol_block()
            .put_block_data_async(genesis.ol_block.clone())
            .await?;
        storage
            .ol_block()
            .set_block_status_async(*genesis.commitment.blkid(), BlockStatus::Valid)
            .await?;
        storage
            .ol_block()
            .replace_canonical_suffix_from_async(0, vec![*genesis.commitment.blkid()])
            .await?;
        storage
            .ol_state()
            .put_toplevel_ol_state_async(genesis.commitment, genesis.ol_state.clone())
            .await?;
        storage
            .ol_checkpoint()
            .insert_epoch_summary_async(genesis.epoch_summary)
            .await?;

        let status = Arc::new(StatusChannel::new(
            ClientState::default(),
            L1BlockCommitment::default(),
            L1Status::default(),
            None,
            None,
        ));

        let config: Config = toml::from_str(
            r#"
            [client]
            rpc_host = "127.0.0.1"
            l2_blocks_fetch_limit = 100
            db_retry_count = 1
            [bitcoind]
            rpc_url = "http://127.0.0.1:1"
            rpc_user = "test"
            rpc_password = "test"
            network = "regtest"
            [btcio]
            l1_reorg_safe_depth = 6
            [btcio.reader]
            client_poll_dur_ms = 200
            [btcio.writer]
            write_poll_dur_ms = 200
            fee_policy = "fixed"
            fixed_fee_rate = 1
            reveal_amount = 100
            bundle_interval_ms = 1000
            [btcio.broadcaster]
            poll_interval_ms = 1000
        "#,
        )?;
        let asm_params = Arc::new(AsmParams {
            magic: MagicBytes::new(*b"ALPN"),
            anchor: L1Anchor {
                block: L1BlockCommitment::default(),
                next_target: 0x207fffff,
                epoch_start_timestamp: 0,
                network: Network::Regtest,
            },
            subprotocols: vec![],
        });

        let client = Arc::new(Client::new(
            "http://127.0.0.1:1".into(),
            Auth::UserPass("test".into(), "test".into()),
            Some(0),
            Some(0),
            None,
        )?);

        let node = NodeContext::new(
            Handle::current(),
            config,
            None,
            asm_params,
            params.clone(),
            storage.clone(),
            client,
            status.clone(),
        );

        let worker = Arc::new(start_chain_worker_service_from_ctx(&node)?);

        // The framework publishes status after commands, not after on_launch.
        timeout(
            Duration::from_secs(5),
            worker.update_safe_tip(genesis.commitment),
        )
        .await??;

        let mut builder = ServiceBuilder::<FixedCsmStatus, _>::new().with_state(FixedCsmStatus);
        let command = builder.create_command_handle(1);
        let monitor: ServiceMonitor<CsmWorkerStatus> =
            builder.launch_sync("test_csm_status", node.executor().as_ref())?;
        drop(command); // The final, immutable status remains readable after this worker exits.

        let ctx = Arc::new(StrataFcmContext::new(
            storage,
            params,
            worker.clone(),
            Arc::new(monitor),
            status,
        ));
        let (secret, public) = test_schnorr_keypair();

        Ok(Self {
            node,
            ctx,
            worker,
            genesis,
            account,
            serial,
            secret,
            predicate: schnorr_predicate(&public),
        })
    }

    fn storage(&self) -> &NodeStorage {
        self.node.storage()
    }

    async fn start_fcm(&self) -> Result<FcmServiceHandle> {
        start_fcm_service(
            self.predicate.clone(),
            self.ctx.clone(),
            self.node.status_channel().subscribe_checkpoint_state(),
            self.node.executor().clone(),
        )
        .await
    }

    fn block(&self, manifest: AsmManifest, terminal: bool) -> OLBlockV1 {
        let mut state = MemoryStateBaseLayer::new(self.genesis.ol_state.clone());
        let components = BlockComponents::new_manifests(vec![manifest]);
        let components = if terminal {
            components.as_terminal()
        } else {
            components
        };

        let completed = execute_block(
            &mut state,
            &BlockInfo::new(1000, 1, 1),
            Some(self.genesis.ol_block.header()),
            components,
        )
        .unwrap();
        let message = Buf32::from(completed.header().compute_blkid());

        OLBlockV1::new(
            SignedOLBlockHeaderV1::new(
                completed.header().clone(),
                sign_schnorr_sig(&message, &self.secret),
            ),
            completed.body().clone(),
        )
    }

    async fn extend_l1(&self, from: u32, to: u32, first_id: L1BlockId) -> Result<()> {
        for height in from..=to {
            let id = if height == from {
                first_id
            } else {
                L1BlockId::from(Buf32::from([height as u8; 32]))
            };
            self.storage()
                .l1()
                .extend_canonical_chain_async(&id, height)
                .await?;
        }

        Ok(())
    }

    async fn assert_unexecuted(&self, block: &OLBlockV1) -> Result<()> {
        let commitment = block.header().compute_block_commitment();

        assert_eq!(
            self.storage()
                .ol_block()
                .get_block_status_async(*commitment.blkid())
                .await?,
            Some(BlockStatus::Unchecked)
        );

        self.assert_no_execution_artifacts(block).await
    }

    async fn assert_no_execution_artifacts(&self, block: &OLBlockV1) -> Result<()> {
        let commitment = block.header().compute_block_commitment();

        assert!(
            self.storage()
                .ol_state()
                .get_write_batch_async(commitment)
                .await?
                .is_none()
        );

        assert!(
            self.storage()
                .ol_state()
                .get_toplevel_ol_state_async(commitment)
                .await?
                .is_none()
        );

        assert!(
            self.storage()
                .ol_state_indexing()
                .get_epoch_indexing_data_async(1)
                .await?
                .is_none()
        );

        assert!(
            self.storage()
                .ol_checkpoint()
                .get_epoch_commitments_at_async(1)
                .await?
                .is_empty()
        );

        assert_eq!(
            self.storage()
                .mmr_index()
                .get_handle(MmrId::L1BlockRefs)
                .get_leaf_count()
                .await?,
            1
        );

        assert_eq!(
            self.storage()
                .mmr_index()
                .get_handle(MmrId::SnarkMsgInbox(self.account))
                .get_leaf_count()
                .await?,
            0
        );

        assert_eq!(self.worker.get_status().cur_tip, self.genesis.commitment);
        assert!(self.node.status_channel().get_ol_sync_status().is_none());

        assert!(
            self.storage()
                .ol_block()
                .get_canonical_block_at_async(1)
                .await?
                .is_none()
        );

        Ok(())
    }
}

async fn wait_until(mut condition: impl AsyncFnMut() -> bool) {
    timeout(Duration::from_secs(10), async {
        while !condition().await {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("service condition should become true");
}

#[test]
fn worker_failures_defer_pending_provenance() {
    let block = OLBlockCommitment::new(1, OLBlockId::from(Buf32::from([1; 32])));

    for reason in [
        ManifestPendingReason::MissingTip,
        ManifestPendingReason::NotBuried,
        ManifestPendingReason::MissingManifest,
    ] {
        assert!(matches!(
            classify_worker_failure(block, WorkerError::ManifestPending { height: 1, reason })
                .unwrap(),
            WorkerFailureOutcome::Deferred(ExecutionDeferral::Dependency)
        ));
    }

    assert!(matches!(
        classify_worker_failure(block, WorkerError::ManifestStorage(DbError::Busy)).unwrap(),
        WorkerFailureOutcome::Deferred(ExecutionDeferral::Storage)
    ));
    assert!(matches!(
        classify_worker_failure(block, WorkerError::MissingPreState(block)).unwrap(),
        WorkerFailureOutcome::Deferred(ExecutionDeferral::Dependency)
    ));
}

#[test]
fn worker_failures_defer_local_storage_errors() {
    let block = OLBlockCommitment::new(1, OLBlockId::from(Buf32::from([1; 32])));
    assert!(matches!(
        classify_worker_failure(block, WorkerError::Database(DbError::Busy)).unwrap(),
        WorkerFailureOutcome::Deferred(ExecutionDeferral::Storage)
    ));
}

#[test]
fn worker_failure_preserves_structured_validation_error() {
    let block = OLBlockCommitment::new(1, OLBlockId::from(Buf32::from([1; 32])));
    let error = ExecError::AsmManifestHeightMismatch {
        expected: 2,
        actual: 3,
        index: 0,
    };

    assert!(matches!(
        classify_worker_failure(block, WorkerError::StfExecution(error)).unwrap(),
        WorkerFailureOutcome::Rejected(ExecError::AsmManifestHeightMismatch {
            expected: 2,
            actual: 3,
            index: 0,
        })
    ));
}

#[test]
fn worker_failure_rejects_confirmed_manifest_mismatch() {
    let block = OLBlockCommitment::new(1, OLBlockId::from(Buf32::from([1; 32])));
    let error = ExecError::AsmManifestContentMismatch { height: 1 };

    assert!(matches!(
        classify_worker_failure(block, WorkerError::StfExecution(error)).unwrap(),
        WorkerFailureOutcome::Rejected(ExecError::AsmManifestContentMismatch { height: 1 })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signed_mismatching_deposit_is_rejected_and_allows_same_slot_replacement() -> Result<()> {
    for terminal in [false, true] {
        let harness = Harness::new().await?;
        let canonical = make_empty_manifest(1, 7);

        let deposit = make_deposit_manifest_for_account(
            1,
            7,
            harness.serial,
            SubjectId::from([0; 32]),
            BitcoinAmount::try_from(100)?,
        );
        let forged = AsmManifest::new(
            1,
            *canonical.blkid(),
            *canonical.wtxids_root(),
            deposit.logs().to_vec(),
        )
        .unwrap();

        let block = harness.block(forged, terminal);
        assert!(verify_sequencer_predicate_signature(
            &harness.predicate,
            &Buf32::from(block.header().compute_blkid()),
            block.signed_header().signature().unwrap()
        ));

        harness.extend_l1(1, 6, *canonical.blkid()).await?;
        harness
            .storage()
            .l1()
            .put_block_data_async(canonical.clone())
            .await?;

        let fcm = harness.start_fcm().await?;
        harness
            .storage()
            .ol_block()
            .put_block_data_with_high_watermark_async(block.clone())
            .await?;

        let commitment = block.header().compute_block_commitment();
        assert!(
            fcm.submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(*commitment.blkid()))
                .await
        );
        wait_until(async || {
            harness
                .storage()
                .ol_block()
                .get_block_high_watermark_async()
                .await
                .unwrap()
                .is_none()
        })
        .await;
        assert_eq!(
            harness
                .storage()
                .ol_block()
                .get_block_status_async(*commitment.blkid())
                .await?,
            Some(BlockStatus::Invalid)
        );
        harness.assert_no_execution_artifacts(&block).await?;

        assert_eq!(
            harness
                .storage()
                .ol_block()
                .get_block_high_watermark_async()
                .await?,
            None
        );

        // Store the same-slot replacement through the sequencer watermark gate.
        let honest = harness.block(canonical, terminal);
        let honest_commitment = honest.header().compute_block_commitment();

        harness
            .storage()
            .ol_block()
            .put_block_data_with_high_watermark_async(honest)
            .await?;
        assert!(
            fcm.submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(*honest_commitment.blkid()))
                .await
        );

        wait_until(async || harness.worker.get_status().cur_tip == honest_commitment).await;

        assert_eq!(
            harness
                .storage()
                .ol_block()
                .get_block_status_async(*honest_commitment.blkid())
                .await?,
            Some(BlockStatus::Valid)
        );

        assert_eq!(
            harness
                .storage()
                .ol_state()
                .get_toplevel_ol_state_async(honest_commitment)
                .await?
                .unwrap()
                .get_account_state(&harness.account)
                .unwrap()
                .balance(),
            BitcoinAmount::default()
        );

        assert!(
            harness
                .storage()
                .ol_state()
                .get_toplevel_ol_state_async(commitment)
                .await?
                .is_none()
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn competing_fork_indexing_defers_without_stopping_the_active_chain() -> Result<()> {
    let harness = Harness::new().await?;
    let signed_empty_block =
        |state: &mut MemoryStateBaseLayer, parent: &OLBlockV1, timestamp, slot| {
            let completed = execute_block(
                state,
                &BlockInfo::new(timestamp, slot, 1),
                Some(parent.header()),
                BlockComponents::new_empty(),
            )
            .unwrap();
            let message = Buf32::from(completed.header().compute_blkid());
            OLBlockV1::new(
                SignedOLBlockHeaderV1::new(
                    completed.header().clone(),
                    sign_schnorr_sig(&message, &harness.secret),
                ),
                completed.body().clone(),
            )
        };

    let mut active_state = MemoryStateBaseLayer::new(harness.genesis.ol_state.clone());
    let active = signed_empty_block(&mut active_state, &harness.genesis.ol_block, 1_000, 1);
    let child = signed_empty_block(&mut active_state, &active, 2_000, 2);
    let mut fork_state = MemoryStateBaseLayer::new(harness.genesis.ol_state.clone());
    let fork = signed_empty_block(&mut fork_state, &harness.genesis.ol_block, 1_001, 1);
    let active_commitment = active.header().compute_block_commitment();
    let fork_commitment = fork.header().compute_block_commitment();
    let child_commitment = child.header().compute_block_commitment();
    let child_state_root = *child.header().state_root();
    assert_ne!(active_commitment.blkid(), fork_commitment.blkid());

    let fcm = harness.start_fcm().await?;
    harness
        .storage()
        .ol_block()
        .put_block_data_async(active)
        .await?;
    assert!(
        fcm.submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(*active_commitment.blkid()))
            .await
    );
    wait_until(async || {
        harness
            .storage()
            .ol_block()
            .get_canonical_block_at_async(1)
            .await
            .unwrap()
            == Some(active_commitment)
    })
    .await;

    harness
        .storage()
        .ol_block()
        .put_block_data_async(fork)
        .await?;
    assert!(
        fcm.submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(*fork_commitment.blkid()))
            .await
    );
    wait_until(async || fcm.fcm_status().pending_blocks() == 1).await;
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_status_async(*fork_commitment.blkid())
            .await?,
        Some(BlockStatus::Unchecked)
    );
    // Execution reaches persistence, but the active fork already owns the epoch index.
    assert!(
        harness
            .storage()
            .ol_state()
            .get_write_batch_async(fork_commitment)
            .await?
            .is_some()
    );
    assert!(
        harness
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(fork_commitment)
            .await?
            .is_none()
    );
    assert_eq!(
        harness
            .storage()
            .ol_state_indexing()
            .get_epoch_indexing_data_async(1)
            .await?
            .unwrap()
            .last_applied_block(),
        Some(&active_commitment)
    );

    harness
        .storage()
        .ol_block()
        .put_block_data_async(child)
        .await?;
    assert!(
        fcm.submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(*child_commitment.blkid()))
            .await
    );
    wait_until(async || {
        harness
            .storage()
            .ol_block()
            .get_canonical_block_at_async(2)
            .await
            .unwrap()
            == Some(child_commitment)
    })
    .await;
    wait_until(async || harness.worker.get_status().cur_tip == child_commitment).await;
    for commitment in [active_commitment, child_commitment] {
        assert_eq!(
            harness
                .storage()
                .ol_block()
                .get_canonical_block_at_async(commitment.slot())
                .await?,
            Some(commitment)
        );
        assert_eq!(
            harness
                .storage()
                .ol_block()
                .get_block_status_async(*commitment.blkid())
                .await?,
            Some(BlockStatus::Valid)
        );
    }
    let stored_state = harness
        .storage()
        .ol_state()
        .get_toplevel_ol_state_async(child_commitment)
        .await?
        .unwrap();
    assert_eq!(
        MemoryStateBaseLayer::new((*stored_state).clone()).compute_state_root()?,
        child_state_root
    );
    assert_eq!(
        harness
            .storage()
            .ol_state_indexing()
            .get_epoch_indexing_data_async(1)
            .await?
            .unwrap()
            .last_applied_block(),
        Some(&child_commitment)
    );
    assert_eq!(fcm.fcm_status().pending_blocks(), 1);
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_status_async(*fork_commitment.blkid())
            .await?,
        Some(BlockStatus::Unchecked)
    );
    assert!(
        harness
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(fork_commitment)
            .await?
            .is_none()
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_replays_unchecked_and_retries_when_asm_output_arrives() -> Result<()> {
    let harness = Harness::new().await?;
    let canonical = make_deposit_manifest_for_account(
        1,
        8,
        harness.serial,
        SubjectId::from([0; 32]),
        BitcoinAmount::try_from(100)?,
    );

    harness.extend_l1(1, 6, *canonical.blkid()).await?;
    let block = harness.block(canonical.clone(), true);
    let commitment = block.header().compute_block_commitment();

    // Persist before service launch, as after a crash between receipt and execution.
    harness
        .storage()
        .ol_block()
        .put_block_data_with_high_watermark_async(block.clone())
        .await?;

    let fcm = harness.start_fcm().await?;
    wait_until(async || fcm.fcm_status().pending_blocks() == 1).await;
    harness.assert_unexecuted(&block).await?;

    harness
        .storage()
        .l1()
        .put_block_data_async(canonical)
        .await?;
    // No second NewBlock message or new epoch is needed: timer retries notice ASM catch-up.
    wait_until(async || harness.worker.get_status().cur_tip == commitment).await;
    wait_until(async || fcm.fcm_status().pending_blocks() == 0).await;

    let state = harness
        .storage()
        .ol_state()
        .get_toplevel_ol_state_async(commitment)
        .await?
        .unwrap();

    assert_eq!(
        state.get_account_state(&harness.account).unwrap().balance(),
        BitcoinAmount::try_from(100)?
    );

    assert_eq!(
        harness
            .storage()
            .mmr_index()
            .get_handle(MmrId::SnarkMsgInbox(harness.account))
            .get_leaf_count()
            .await?,
        1
    );

    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_status_async(*commitment.blkid())
            .await?,
        Some(BlockStatus::Valid)
    );

    drop(fcm);
    let restored = harness.start_fcm().await?;
    assert_eq!(restored.fcm_status().pending_blocks(), 0);

    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_canonical_block_at_async(1)
            .await?,
        Some(commitment)
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_preserves_indexes_until_reorg_replacement_is_buried() -> Result<()> {
    let harness = Harness::new().await?;
    let canonical = make_empty_manifest(1, 7);

    harness.extend_l1(1, 6, *canonical.blkid()).await?;
    harness
        .storage()
        .l1()
        .put_block_data_async(canonical.clone())
        .await?;

    let block = harness.block(canonical.clone(), false);
    let commitment = block.header().compute_block_commitment();
    let fcm = harness.start_fcm().await?;
    harness
        .storage()
        .ol_block()
        .put_block_data_async(block)
        .await?;
    assert!(
        fcm.submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(*commitment.blkid()))
            .await
    );
    wait_until(async || {
        matches!(
            harness
                .storage()
                .ol_block()
                .get_block_status_async(*commitment.blkid())
                .await,
            Ok(Some(BlockStatus::Valid))
        )
    })
    .await;
    wait_until(async || {
        matches!(
            harness.storage().ol_block().get_canonical_block_at_async(1).await,
            Ok(Some(tip)) if tip == commitment
        )
    })
    .await;
    drop(fcm);

    harness
        .storage()
        .l1()
        .revert_canonical_chain_async(0)
        .await?;

    let l1_refs = harness.storage().mmr_index().get_handle(MmrId::L1BlockRefs);
    let original_leaf_count = l1_refs.get_leaf_count().await?;
    let startup = harness.start_fcm();
    pin!(startup);
    assert!(
        timeout(Duration::from_millis(250), startup.as_mut())
            .await
            .is_err()
    );
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_status_async(*commitment.blkid())
            .await?,
        Some(BlockStatus::Valid)
    );
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_canonical_block_at_async(1)
            .await?,
        Some(commitment)
    );
    assert_eq!(l1_refs.get_leaf_count().await?, original_leaf_count);

    harness.extend_l1(1, 6, *canonical.blkid()).await?;
    let restarted = timeout(Duration::from_secs(10), startup).await??;
    assert_eq!(restarted.fcm_status().pending_blocks(), 0);
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_status_async(*commitment.blkid())
            .await?,
        Some(BlockStatus::Valid)
    );
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_canonical_block_at_async(1)
            .await?,
        Some(commitment)
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shallow_l1_branch_change_accepts_only_the_replacement_after_burial() -> Result<()> {
    let harness = Harness::new().await?;
    let old = make_empty_manifest(1, 1);
    let canonical = make_empty_manifest(1, 2);

    // Both manifests can exist by ID; only the canonical, buried one authenticates.
    harness
        .storage()
        .l1()
        .put_block_data_async(old.clone())
        .await?;
    harness
        .storage()
        .l1()
        .put_block_data_async(canonical.clone())
        .await?;

    harness
        .extend_l1(0, 0, L1BlockId::from(Buf32::zero()))
        .await?;
    harness.extend_l1(1, 2, *old.blkid()).await?;
    let stale = harness.block(old, false);
    let replacement = harness.block(canonical.clone(), false);
    let commitment = replacement.header().compute_block_commitment();

    harness
        .storage()
        .ol_block()
        .put_block_data_async(stale.clone())
        .await?;
    harness
        .storage()
        .ol_block()
        .put_block_data_async(replacement.clone())
        .await?;

    let fcm = harness.start_fcm().await?;
    wait_until(async || fcm.fcm_status().pending_blocks() == 2).await;
    harness.assert_unexecuted(&stale).await?;
    harness.assert_unexecuted(&replacement).await?;

    // Reorg two shallow blocks, within depth six. No accepted prefix is reverted.
    harness
        .storage()
        .l1()
        .revert_canonical_chain_async(0)
        .await?;
    harness.extend_l1(1, 6, *canonical.blkid()).await?;
    wait_until(async || harness.worker.get_status().cur_tip == commitment).await;
    // FCM records the canonical suffix after updating the worker's safe tip.
    wait_until(async || {
        matches!(
            harness.storage().ol_block().get_canonical_block_at_async(1).await,
            Ok(Some(tip)) if tip == commitment
        )
    })
    .await;
    wait_until(async || {
        harness
            .storage()
            .ol_block()
            .get_block_status_async(stale.header().compute_blkid())
            .await
            .unwrap()
            == Some(BlockStatus::Invalid)
    })
    .await;

    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_status_async(stale.header().compute_blkid())
            .await?,
        Some(BlockStatus::Invalid)
    );

    assert!(
        harness
            .storage()
            .ol_state()
            .get_write_batch_async(stale.header().compute_block_commitment())
            .await?
            .is_none()
    );

    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_canonical_block_at_async(1)
            .await?,
        Some(commitment)
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_rejects_stored_valid_mismatch_before_reconciliation() -> Result<()> {
    let harness = Harness::new().await?;
    let canonical = make_empty_manifest(1, 7);
    let forged = make_deposit_manifest_for_account(
        1,
        7,
        harness.serial,
        SubjectId::from([0; 32]),
        BitcoinAmount::try_from(100)?,
    );
    let block = harness.block(forged, true);
    let commitment = block.header().compute_block_commitment();

    harness.extend_l1(1, 6, *canonical.blkid()).await?;
    harness
        .storage()
        .l1()
        .put_block_data_async(canonical)
        .await?;
    harness
        .storage()
        .ol_block()
        .put_block_data_with_high_watermark_async(block)
        .await?;
    harness
        .storage()
        .ol_block()
        .set_block_status_async(*commitment.blkid(), BlockStatus::Valid)
        .await?;
    harness
        .storage()
        .ol_block()
        .replace_canonical_suffix_from_async(1, vec![*commitment.blkid()])
        .await?;

    let error = harness
        .start_fcm()
        .await
        .expect_err("mismatch must reject startup");
    assert!(matches!(
        error.downcast_ref::<ExecError>(),
        Some(ExecError::AsmManifestContentMismatch { height: 1 })
    ));

    // Startup authentication is read-only: do not reconcile or trust an invalid stored tip.
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_status_async(*commitment.blkid())
            .await?,
        Some(BlockStatus::Valid)
    );
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_canonical_block_at_async(1)
            .await?,
        Some(commitment)
    );
    assert_eq!(
        harness
            .storage()
            .ol_block()
            .get_block_high_watermark_async()
            .await?,
        Some(commitment)
    );
    assert!(
        harness
            .storage()
            .ol_state()
            .get_write_batch_async(commitment)
            .await?
            .is_none()
    );
    assert_eq!(
        harness.worker.get_status().cur_tip,
        harness.genesis.commitment
    );
    assert!(harness.node.status_channel().get_ol_sync_status().is_none());

    Ok(())
}
