use std::{collections::BTreeMap, sync::Arc};

use jsonrpsee::{
    core::{RpcResult, server::MethodsError},
    rpc_params,
};
use strata_asm_checkpoint_types::{
    CheckpointPayload, CheckpointTip, test_utils::create_test_checkpoint_payload,
};
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint, L1Status};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_tests::asm_tests::make_test_asm_state;
use strata_db_types::ol_state_index::IndexingWrites;
use strata_identifiers::{
    AccountId, Buf32, Buf64, Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment,
    OLBlockId, RBuf32, Slot,
};
use strata_ol_canonical_chain_provider::{FinalizedOLChainProvider, OLCanonicalChainProviderImpl};
use strata_ol_chain_types_v1::{
    BlockFlagsV1, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLTxSegmentV1, SignedOLBlockHeaderV1,
};
use strata_ol_rpc_types::{OLRpcProvider, RpcCheckpointInfo, RpcOLBlockSummary};
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::{NodeStorage, create_node_storage, test_runtime_handle};

use super::{
    RpcDeps, build_public_rpc_module, build_submit_rpc_module,
    errors::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    provider::NodeRpcProvider,
};
use crate::run_context::NodeRole;

struct RpcFixture {
    storage: Arc<NodeStorage>,
    genesis: EpochCommitment,
    genesis_l1: L1BlockCommitment,
}

impl RpcFixture {
    fn new() -> Self {
        let genesis = make_block(0, 0, OLBlockId::from(Buf32::zero()), true, 0);
        let fixture = Self {
            storage: Arc::new(
                create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap(),
            ),
            genesis: EpochCommitment::from_terminal(0, genesis.header().compute_block_commitment()),
            genesis_l1: L1BlockCommitment::new(100, Buf32::from([100; 32]).into()),
        };
        fixture
            .storage
            .ol_block()
            .put_block_data_blocking(genesis)
            .unwrap();
        fixture.store_summary(fixture.genesis, EpochCommitment::null());
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(0, vec![*fixture.genesis.last_blkid()])
            .unwrap();
        fixture
            .storage
            .l1()
            .extend_canonical_chain(fixture.genesis_l1.blkid(), 100)
            .unwrap();
        fixture
    }

    fn genesis_header(&self) -> OLBlockHeaderV1 {
        self.storage
            .ol_block()
            .get_ol_header_blocking(*self.genesis.last_blkid())
            .unwrap()
            .unwrap()
    }

    fn append_block(
        &self,
        parent: &OLBlockHeaderV1,
        terminal: bool,
        branch: u8,
    ) -> OLBlockHeaderV1 {
        let block = make_block(
            parent.slot() + 1,
            parent.epoch() + u32::from(parent.is_terminal()),
            parent.compute_blkid(),
            terminal,
            branch,
        );
        let header = block.header().clone();
        self.storage
            .ol_block()
            .put_block_data_blocking(block)
            .unwrap();
        header
    }

    fn checkpoint_info(&self, epoch: Epoch) -> RpcResult<Option<RpcCheckpointInfo>> {
        let module = build_public_rpc_module(&self.deps(NodeRole::Sequencer)).unwrap();
        test_runtime_handle()
            .block_on(module.call("strata_getCheckpointInfo", rpc_params![epoch]))
            .map_err(|error| match error {
                MethodsError::JsonRpc(error) => error,
                error => panic!("unexpected RPC invocation failure: {error}"),
            })
    }

    fn store_summary(&self, commitment: EpochCommitment, previous: EpochCommitment) {
        self.storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(EpochSummary::new(
                commitment.epoch(),
                commitment.to_block_commitment(),
                previous.to_block_commitment(),
                self.genesis_l1,
                Buf32::from([1; 32]),
            ))
            .unwrap();
    }

    fn finalize_first_epoch(&self) -> EpochCommitment {
        let commitment = EpochCommitment::new(1, 3, Buf32::from([2; 32]).into());
        let observed_at = L1BlockCommitment::new(101, Buf32::from([101; 32]).into());
        self.storage
            .l1()
            .extend_canonical_chain(observed_at.blkid(), observed_at.height())
            .unwrap();
        self.storage
            .asm()
            .put_state_blocking(observed_at, make_test_asm_state())
            .unwrap();
        let reference =
            CheckpointL1Ref::new(observed_at, RBuf32::from([2; 32]), RBuf32::from([3; 32]));
        let tip = CheckpointTip::new(1, 101, commitment.to_block_commitment());
        let sample = create_test_checkpoint_payload(1);
        let payload =
            CheckpointPayload::new(tip, sample.sidecar().clone(), sample.proof().to_vec()).unwrap();
        self.storage
            .ol_checkpoint()
            .put_checkpoint_l1_observation_blocking(commitment, payload, reference.clone())
            .unwrap();
        let checkpoint = L1Checkpoint::new(tip, reference);
        self.storage
            .client_state()
            .put_update_blocking(
                &observed_at,
                ClientUpdateOutput::new_state(ClientState::new(
                    Some(checkpoint.clone()),
                    Some(checkpoint),
                )),
            )
            .unwrap();
        commitment
    }

    fn deps(&self, node_role: NodeRole) -> RpcDeps {
        RpcDeps {
            rpc_host: "127.0.0.1".into(),
            rpc_port: 0,
            admin_rpc_host: "127.0.0.1".into(),
            admin_rpc_port: 0,
            admin_rpc_bearer_token: None,
            submit_rpc_host: "127.0.0.1".into(),
            submit_rpc_port: 0,
            submit_rpc_bearer_token: None,
            genesis_l1_block: self.genesis_l1,
            max_headers_range: 100,
            node_role,
            storage: self.storage.clone(),
            status_channel: Arc::new(StatusChannel::new(
                ClientState::new(None, None),
                self.genesis_l1,
                L1Status::default(),
                None,
                None,
            )),
            mempool_handle: None,
            #[cfg(feature = "sequencer")]
            seq_deps: None,
        }
    }
}

fn make_block(
    slot: Slot,
    epoch: Epoch,
    parent: OLBlockId,
    terminal: bool,
    branch: u8,
) -> OLBlockV1 {
    let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![]).unwrap());
    let mut flags = BlockFlagsV1::zero();
    flags.set_is_terminal(terminal);
    let header = OLBlockHeaderV1::new(
        slot,
        flags,
        slot,
        epoch,
        parent,
        body.compute_hash_commitment(),
        Buf32::from([branch; 32]),
        Buf32::zero(),
    );
    OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body)
}

#[test]
fn test_recent_blocks_uses_canonical_branch_before_status_publication() {
    let fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let pivot = fixture.append_block(&genesis, false, 0);
    let branch_a = fixture.append_block(&pivot, true, 1);
    let branch_b = fixture.append_block(&pivot, true, 2);
    let tip_b = fixture.append_block(&branch_b, false, 2);
    let deps = fixture.deps(NodeRole::Sequencer);
    deps.status_channel
        .update_ol_sync_status(OLSyncStatusUpdate::new(OLSyncStatus::new(
            branch_a.compute_block_commitment(),
            branch_a.epoch(),
            true,
            EpochCommitment::from_terminal(branch_a.epoch(), branch_a.compute_block_commitment()),
            fixture.genesis,
            fixture.genesis,
            fixture.genesis_l1,
        )));
    // FCM has published B's index suffix; status still describes branch A.
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(
            1,
            vec![
                pivot.compute_blkid(),
                branch_b.compute_blkid(),
                tip_b.compute_blkid(),
            ],
        )
        .unwrap();
    let module = build_public_rpc_module(&deps).unwrap();
    let summaries: Vec<RpcOLBlockSummary> = test_runtime_handle()
        .block_on(module.call("strata_getRecentBlocks", rpc_params![3]))
        .unwrap();
    assert_eq!(
        summaries
            .iter()
            .map(|block| block.blkid())
            .collect::<Vec<_>>(),
        vec![
            pivot.compute_blkid(),
            branch_b.compute_blkid(),
            tip_b.compute_blkid()
        ]
    );
}

#[test]
fn test_recent_blocks_requires_bodies_after_header_traversal() {
    let fixture = RpcFixture::new();
    let first = fixture.append_block(&fixture.genesis_header(), true, 1);
    let tip = fixture.append_block(&first, false, 1);
    fixture
        .storage
        .ol_block()
        .put_terminal_header_blocking(first.compute_blkid(), first.clone())
        .unwrap();
    fixture
        .storage
        .ol_block()
        .del_block_data_blocking(first.compute_blkid())
        .unwrap();
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(1, vec![first.compute_blkid(), tip.compute_blkid()])
        .unwrap();
    let module = build_public_rpc_module(&fixture.deps(NodeRole::Sequencer)).unwrap();
    let error = test_runtime_handle()
        .block_on(
            module.call::<_, Vec<RpcOLBlockSummary>>("strata_getRecentBlocks", rpc_params![2]),
        )
        .unwrap_err();
    let MethodsError::JsonRpc(error) = error else {
        panic!("expected block-read RPC error")
    };
    assert_eq!(error.code(), INVALID_PARAMS_CODE);
    assert_eq!(
        error.message(),
        format!("block not found: {}", first.compute_blkid())
    );
}

#[test]
fn test_checkpoint_first_block_follows_selected_branch_across_reorg() {
    let fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let pivot = fixture.append_block(&genesis, false, 0);
    let previous_a = fixture.append_block(&pivot, true, 1);
    let first_a = fixture.append_block(&previous_a, false, 1);
    let terminal_a = fixture.append_block(&first_a, true, 1);
    let fork = fixture.append_block(&pivot, false, 2);
    let previous_b = fixture.append_block(&fork, true, 2);
    let first_b = fixture.append_block(&previous_b, false, 2);
    let terminal_b = fixture.append_block(&first_b, true, 2);
    for (previous, terminal) in [(&previous_a, &terminal_a), (&previous_b, &terminal_b)] {
        let previous_epoch = EpochCommitment::from_terminal(1, previous.compute_block_commitment());
        fixture.store_summary(previous_epoch, fixture.genesis);
        fixture.store_summary(
            EpochCommitment::from_terminal(2, terminal.compute_block_commitment()),
            previous_epoch,
        );
    }
    for (chain, first, terminal) in [
        (
            vec![&pivot, &previous_a, &first_a, &terminal_a],
            &first_a,
            &terminal_a,
        ),
        (
            vec![&pivot, &fork, &previous_b, &first_b, &terminal_b],
            &first_b,
            &terminal_b,
        ),
    ] {
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(
                1,
                chain
                    .into_iter()
                    .map(OLBlockHeaderV1::compute_blkid)
                    .collect(),
            )
            .unwrap();
        let info = fixture.checkpoint_info(2).unwrap().unwrap();
        assert_eq!(info.l2_start, Some(first.compute_block_commitment()));
        assert_eq!(info.l2_end, terminal.compute_block_commitment());
    }
}

#[test]
fn test_checkpoint_first_block_rejects_exact_predecessor_mismatch() {
    let fixture = RpcFixture::new();
    let previous = fixture.append_block(&fixture.genesis_header(), true, 1);
    let terminal = fixture.append_block(&previous, true, 1);
    let commitment = EpochCommitment::from_terminal(2, terminal.compute_block_commitment());
    let wrong = OLBlockCommitment::new(previous.slot() + 1, previous.compute_blkid());
    fixture.store_summary(commitment, EpochCommitment::from_terminal(1, wrong));
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(terminal.slot(), vec![terminal.compute_blkid()])
        .unwrap();
    let error = fixture.checkpoint_info(2).unwrap_err();
    assert_eq!(error.code(), INTERNAL_ERROR_CODE);
    assert_eq!(
        error.message(),
        format!(
            "previous terminal block mismatch (expected {}, got {})",
            wrong,
            previous.compute_block_commitment()
        )
    );
}

#[test]
fn test_checkpoint_first_block_preserves_missing_ancestor_error() {
    let fixture = RpcFixture::new();
    let first = fixture.append_block(&fixture.genesis_header(), false, 1);
    let terminal = fixture.append_block(&first, true, 1);
    fixture.store_summary(
        EpochCommitment::from_terminal(1, terminal.compute_block_commitment()),
        fixture.genesis,
    );
    fixture
        .storage
        .ol_block()
        .del_block_data_blocking(first.compute_blkid())
        .unwrap();
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(terminal.slot(), vec![terminal.compute_blkid()])
        .unwrap();
    let error = fixture.checkpoint_info(1).unwrap_err();
    assert_eq!(error.code(), INVALID_PARAMS_CODE);
    assert_eq!(
        error.message(),
        format!("block not found: {}", first.compute_blkid())
    );
}

#[test]
fn test_checkpoint_first_block_rejects_invalid_summary_terminal() {
    for terminal_flag in [true, false] {
        let fixture = RpcFixture::new();
        let terminal = fixture.append_block(&fixture.genesis_header(), terminal_flag, 1);
        let epoch = if terminal_flag { 2 } else { 1 };
        let commitment = EpochCommitment::from_terminal(epoch, terminal.compute_block_commitment());
        fixture.store_summary(commitment, fixture.genesis);
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(terminal.slot(), vec![terminal.compute_blkid()])
            .unwrap();
        let error = fixture.checkpoint_info(epoch).unwrap_err();
        assert_eq!(error.code(), INTERNAL_ERROR_CODE);
        let expected = if terminal_flag {
            format!(
                "epoch commitment mismatch (expected {}, got {})",
                commitment,
                EpochCommitment::from_terminal(
                    terminal.epoch(),
                    terminal.compute_block_commitment()
                )
            )
        } else {
            format!(
                "expected terminal block flag on block {}",
                terminal.compute_block_commitment()
            )
        };
        assert_eq!(error.message(), expected);
    }
}

#[test]
fn test_css_rpc_resolves_finality_before_summary_application() {
    let fixture = RpcFixture::new();
    // L1 finality advances while the OL index and local summaries remain at genesis.
    let finalized = fixture.finalize_first_epoch();
    let deps = fixture.deps(NodeRole::CheckpointSync);
    let provider = NodeRpcProvider::new(
        fixture.storage.clone(),
        FinalizedOLChainProvider::new(&fixture.storage),
        deps.status_channel.clone(),
        None,
    );
    let module = build_public_rpc_module(&deps).unwrap();
    test_runtime_handle().block_on(async {
        assert_eq!(
            provider.get_canonical_epoch_commitment_at(1).await.unwrap(),
            Some(finalized)
        );
        assert_eq!(
            provider.get_canonical_block_at(3).await.unwrap(),
            Some(finalized.to_block_commitment())
        );
        assert_eq!(provider.get_canonical_block_at(2).await.unwrap(), None);
        let info: Option<RpcCheckpointInfo> = module
            .call("strata_getCheckpointInfo", rpc_params![1])
            .await
            .unwrap();
        assert!(info.is_none());
    });

    // Applying the exact summary makes the summary-dependent RPC available, without bodies.
    fixture.store_summary(finalized, fixture.genesis);
    let info: Option<RpcCheckpointInfo> = test_runtime_handle()
        .block_on(module.call("strata_getCheckpointInfo", rpc_params![1]))
        .unwrap();
    let info = info.unwrap();
    assert_eq!(info.l2_end, finalized.to_block_commitment());
    assert_eq!(info.l2_start, None);
    assert_eq!(
        fixture
            .storage
            .ol_block()
            .get_canonical_tip_blocking()
            .unwrap(),
        Some(fixture.genesis.to_block_commitment())
    );
    assert!(
        !module
            .method_names()
            .any(|name| name == "strata_getBlockBySlot")
    );
}

#[test]
fn test_rpc_roles_select_index_or_finalized_epochs() {
    let fixture = RpcFixture::new();
    let finalized = fixture.finalize_first_epoch();
    fixture.store_summary(finalized, fixture.genesis);
    let orphan = EpochCommitment::new(2, 5, Buf32::from([3; 32]).into());
    let canonical = EpochCommitment::new(2, 6, Buf32::from([4; 32]).into());
    fixture.store_summary(orphan, finalized);
    fixture.store_summary(canonical, finalized);
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(6, vec![*canonical.last_blkid()])
        .unwrap();
    let account = AccountId::from([5; 32]);
    fixture
        .storage
        .ol_state_indexing()
        .apply_epoch_indexing_blocking(
            canonical,
            IndexingWrites::new(vec![account], BTreeMap::new(), BTreeMap::new()),
        )
        .unwrap();

    // Both block-executing roles select the indexed candidate, not the first stored one.
    for role in [NodeRole::Sequencer, NodeRole::FullNode] {
        let deps = fixture.deps(role);
        let module = build_public_rpc_module(&deps).unwrap();
        let commitment: EpochCommitment = test_runtime_handle()
            .block_on(module.call(
                "strata_getAccountGenesisEpochCommitment",
                rpc_params![account],
            ))
            .unwrap();
        assert_eq!(commitment, canonical);
        assert!(
            module
                .method_names()
                .any(|name| name == "strata_getBlockBySlot")
        );
        assert!(
            !module
                .method_names()
                .any(|name| name == "strata_submitTransaction")
        );
        let provider = NodeRpcProvider::new(
            fixture.storage.clone(),
            OLCanonicalChainProviderImpl::new(&fixture.storage),
            deps.status_channel,
            None,
        );
        assert_eq!(
            test_runtime_handle()
                .block_on(provider.get_canonical_block_at(6))
                .unwrap(),
            Some(canonical.to_block_commitment())
        );
    }

    // CSS stays at the declared finality bound even when newer summaries and index entries exist.
    let module = build_public_rpc_module(&fixture.deps(NodeRole::CheckpointSync)).unwrap();
    let info: Option<RpcCheckpointInfo> = test_runtime_handle()
        .block_on(module.call("strata_getCheckpointInfo", rpc_params![2]))
        .unwrap();
    assert!(info.is_none());

    let submit = build_submit_rpc_module(&fixture.deps(NodeRole::Sequencer)).unwrap();
    assert_eq!(
        submit.method_names().collect::<Vec<_>>(),
        vec!["strata_submitTransaction"]
    );
}
