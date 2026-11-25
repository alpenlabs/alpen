mod archive_import;

use std::{collections::BTreeMap, sync::Arc};

use jsonrpc_core::{RpcResult, server::MethodsError};
use jsonrpsee::{RpcModule, core as jsonrpc_core, rpc_params};
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{ClientState, L1Status};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_tests::asm_tests::make_test_asm_state;
use strata_db_types::ol_state_index::IndexingWrites;
use strata_identifiers::{
    AccountId, Buf32, Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment,
};
use strata_ol_block_ancestry_provider::{BlockAncestryError, test_utils::BlockAncestryView};
use strata_ol_canonical_chain_provider::{
    FinalizedOLChainProvider, OLCanonicalChainProvider, OLCanonicalChainProviderImpl,
    test_utils::{build_checkpoint_payload, finalize_epoch, record_l1_observation},
};
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_ol_rpc_types::{OLRpcProvider, RpcCheckpointInfo, RpcOLBlockSummary};
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::{NodeStorage, create_node_storage, test_runtime_handle};

use super::{
    RpcDeps, build_public_rpc_module, build_submit_rpc_module,
    errors::{FINALIZED_CLIENT_STATE_UNAVAILABLE_CODE, INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    provider::NodeRpcProvider,
};
use crate::run_context::NodeRole;

struct RpcFixture {
    /// Node storage the RPC modules and providers read.
    storage: Arc<NodeStorage>,

    /// Coherent block history backed by `storage.ol_block()`.
    block_history: BlockAncestryView,

    /// Genesis epoch.
    genesis_epoch: EpochCommitment,

    /// L1 block OL genesis anchors to.
    genesis_l1: L1BlockCommitment,

    /// Shared by every module and provider, as on a node.
    status_channel: Arc<StatusChannel>,
}

impl RpcFixture {
    /// Creates a fixture storing the genesis block and summary, its slot-zero canonical OL
    /// entry, and `genesis_l1` as the canonical L1 block at height 100.
    fn new() -> Self {
        let genesis_l1 = L1BlockCommitment::new(100, Buf32::from([100; 32]).into());
        let storage =
            Arc::new(create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap());
        let block_history = BlockAncestryView::with_storage(storage.ol_block().clone());
        let genesis = block_history.genesis().clone();
        let fixture = Self {
            storage,
            block_history,
            genesis_epoch: EpochCommitment::from_terminal(0, genesis.compute_block_commitment()),
            genesis_l1,
            status_channel: Arc::new(StatusChannel::new(
                ClientState::new(None, None),
                genesis_l1,
                L1Status::default(),
                None,
                None,
            )),
        };
        fixture.store_summary(fixture.genesis_epoch, EpochCommitment::null());
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(0, vec![*fixture.genesis_epoch.last_blkid()])
            .unwrap();
        fixture
            .storage
            .l1()
            .extend_canonical_chain(fixture.genesis_l1.blkid(), 100)
            .unwrap();
        fixture
    }

    fn genesis_header(&self) -> OLBlockHeaderV1 {
        self.block_history.genesis().clone()
    }

    fn append_block(&mut self, parent: &OLBlockHeaderV1) -> OLBlockHeaderV1 {
        self.block_history.append_block(parent)
    }

    fn append_terminal_block(&mut self, parent: &OLBlockHeaderV1) -> OLBlockHeaderV1 {
        self.block_history.append_terminal_block(parent)
    }

    fn drop_body(&mut self, block: &OLBlockHeaderV1) {
        self.block_history.drop_body(block.compute_blkid());
    }

    fn drop_header(&mut self, block: &OLBlockHeaderV1) {
        self.block_history.drop_header(block.compute_blkid());
    }

    fn promote_history_base(&mut self, terminal: &OLBlockHeaderV1) -> EpochCommitment {
        self.block_history.promote_history_anchor(terminal)
    }

    fn get_checkpoint_info(&self, epoch: Epoch) -> RpcResult<Option<RpcCheckpointInfo>> {
        test_runtime_handle()
            .block_on(
                self.public_module(NodeRole::Sequencer)
                    .call("strata_getCheckpointInfo", rpc_params![epoch]),
            )
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
        let checkpoint = record_l1_observation(
            &self.storage,
            commitment,
            build_checkpoint_payload(commitment, observed_at.height()),
            observed_at,
        );
        self.storage
            .asm()
            .put_state_blocking(observed_at, make_test_asm_state())
            .unwrap();
        finalize_epoch(&self.storage, checkpoint, observed_at);
        commitment
    }

    /// Builds the public RPC module served by a node with `node_role`.
    fn public_module(&self, node_role: NodeRole) -> RpcModule<()> {
        build_public_rpc_module(&self.make_deps(node_role)).expect("test: build public RPC module")
    }

    /// Builds the sequencer's submit RPC module.
    fn submit_module(&self) -> RpcModule<()> {
        build_submit_rpc_module(&self.make_deps(NodeRole::Sequencer))
            .expect("test: build submit RPC module")
    }

    /// Builds the RPC provider over `canonical_chain_provider`, sharing the modules' status
    /// channel.
    fn node_provider<P: OLCanonicalChainProvider>(
        &self,
        canonical_chain_provider: P,
    ) -> NodeRpcProvider<P> {
        NodeRpcProvider::new(
            self.storage.clone(),
            canonical_chain_provider,
            self.status_channel.clone(),
            None,
        )
    }

    /// Returns RPC dependencies for a node with `node_role`, over the fixture's storage and
    /// status channel.
    fn make_deps(&self, node_role: NodeRole) -> RpcDeps {
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
            history_archive: None,
            node_role,
            storage: self.storage.clone(),
            status_channel: self.status_channel.clone(),
            mempool_handle: None,
            #[cfg(feature = "sequencer")]
            seq_deps: None,
        }
    }
}

#[test]
fn test_recent_blocks_follow_canonical_tip_not_sync_status() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let pivot = fixture.append_block(&genesis);
    let branch_a = fixture.append_terminal_block(&pivot);
    let branch_b = fixture.append_terminal_block(&pivot);
    let tip_b = fixture.append_block(&branch_b);

    // The canonical index already selects branch B while sync status still names branch A.
    fixture
        .status_channel
        .update_ol_sync_status(OLSyncStatusUpdate::new(OLSyncStatus::new(
            branch_a.compute_block_commitment(),
            branch_a.epoch(),
            true,
            EpochCommitment::from_terminal(branch_a.epoch(), branch_a.compute_block_commitment()),
            fixture.genesis_epoch,
            fixture.genesis_epoch,
            fixture.genesis_l1,
        )));
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
    // The RPC must start from the canonical index rather than the stale status tip.
    let summaries: Vec<RpcOLBlockSummary> = test_runtime_handle()
        .block_on(
            fixture
                .public_module(NodeRole::Sequencer)
                .call("strata_getRecentBlocks", rpc_params![3]),
        )
        .unwrap();
    // The returned history is branch B's canonical suffix in execution order.
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
fn test_missing_body_after_header_traversal_returns_not_found() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let first = fixture.append_terminal_block(&genesis);
    let tip = fixture.append_block(&first);

    // Keep the first block's header for ancestry traversal while removing its body.
    fixture.drop_body(&first);
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(1, vec![first.compute_blkid(), tip.compute_blkid()])
        .unwrap();
    // Header traversal succeeds, but rendering the recent block requires the missing body.
    let error = test_runtime_handle()
        .block_on(
            fixture
                .public_module(NodeRole::Sequencer)
                .call::<_, Vec<RpcOLBlockSummary>>("strata_getRecentBlocks", rpc_params![2]),
        )
        .unwrap_err();
    let MethodsError::JsonRpc(error) = error else {
        panic!("expected block-read RPC error")
    };
    // Body absence is exposed as the existing block-not-found RPC response.
    assert_eq!(error.code(), INVALID_PARAMS_CODE);
    assert_eq!(
        error.message(),
        format!("block not found: {}", first.compute_blkid())
    );
}

#[test]
fn test_recent_blocks_exclude_history_base_and_pruned_prefix() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let history_base = fixture.append_terminal_block(&genesis);
    let first = fixture.append_block(&history_base);
    let tip = fixture.append_block(&first);

    // Promotion retains only the base header below the two newer full blocks.
    fixture.promote_history_base(&history_base);
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(
            first.slot(),
            vec![first.compute_blkid(), tip.compute_blkid()],
        )
        .unwrap();

    // The production provider must stop before the history base without reading older data.
    let summaries: Vec<RpcOLBlockSummary> = test_runtime_handle()
        .block_on(
            fixture
                .public_module(NodeRole::Sequencer)
                .call("strata_getRecentBlocks", rpc_params![10]),
        )
        .unwrap();

    assert_eq!(
        summaries
            .iter()
            .map(RpcOLBlockSummary::blkid)
            .collect::<Vec<_>>(),
        vec![first.compute_blkid(), tip.compute_blkid()]
    );
}

#[test]
fn test_checkpoint_start_changes_with_selected_branch_after_reorg() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let pivot = fixture.append_block(&genesis);
    let previous_a = fixture.append_terminal_block(&pivot);
    let first_a = fixture.append_block(&previous_a);
    let terminal_a = fixture.append_terminal_block(&first_a);
    let fork = fixture.append_block(&pivot);
    let previous_b = fixture.append_terminal_block(&fork);
    let first_b = fixture.append_block(&previous_b);
    let terminal_b = fixture.append_terminal_block(&first_b);

    // Both branches have exact summaries for epoch 2 but different first and terminal blocks.
    for (previous, terminal) in [(&previous_a, &terminal_a), (&previous_b, &terminal_b)] {
        let previous_epoch = EpochCommitment::from_terminal(1, previous.compute_block_commitment());
        fixture.store_summary(previous_epoch, fixture.genesis_epoch);
        fixture.store_summary(
            EpochCommitment::from_terminal(2, terminal.compute_block_commitment()),
            previous_epoch,
        );
    }
    // Select each branch in turn and query the same checkpoint epoch.
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
        let info = fixture.get_checkpoint_info(2).unwrap().unwrap();
        // The start and end must follow the branch selected by the canonical index.
        assert_eq!(info.l2_start, Some(first.compute_block_commitment()));
        assert_eq!(info.l2_end, terminal.compute_block_commitment());
    }
}

#[test]
fn test_checkpoint_predecessor_mismatch_returns_internal_error() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let previous = fixture.append_terminal_block(&genesis);
    let terminal = fixture.append_terminal_block(&previous);
    let commitment = EpochCommitment::from_terminal(2, terminal.compute_block_commitment());

    // The summary names the right predecessor ID at the wrong slot.
    let wrong = OLBlockCommitment::new(previous.slot() + 1, previous.compute_blkid());
    fixture.store_summary(commitment, EpochCommitment::from_terminal(1, wrong));
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(terminal.slot(), vec![terminal.compute_blkid()])
        .unwrap();
    // The RPC compares the ancestry walk's exact commitment with the summary predecessor.
    let error = fixture.get_checkpoint_info(2).unwrap_err();

    // A server-selected inconsistent summary is an internal error, not client input failure.
    assert_eq!(error.code(), INTERNAL_ERROR_CODE);
    let expected = BlockAncestryError::PreviousTerminalMismatch {
        expected: wrong,
        actual: previous.compute_block_commitment(),
    };
    assert_eq!(error.message(), expected.to_string());
}

#[test]
fn test_missing_checkpoint_ancestor_returns_internal_error() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let first = fixture.append_block(&genesis);
    let terminal = fixture.append_terminal_block(&first);
    fixture.store_summary(
        EpochCommitment::from_terminal(1, terminal.compute_block_commitment()),
        fixture.genesis_epoch,
    );

    // Remove the intermediate header required to reach epoch 1's first block.
    fixture.drop_header(&first);
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(terminal.slot(), vec![terminal.compute_blkid()])
        .unwrap();
    // The production ancestry provider must preserve the missing-header failure through RPC.
    let error = fixture.get_checkpoint_info(1).unwrap_err();

    assert_eq!(error.code(), INTERNAL_ERROR_CODE);
    let expected = BlockAncestryError::MissingHeader(first.compute_blkid());
    assert_eq!(error.message(), expected.to_string());
}

#[test]
fn test_checkpoint_epoch_mismatch_returns_internal_error() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let terminal = fixture.append_terminal_block(&genesis);

    // Store a summary that assigns the terminal to an epoch other than its header's.
    let commitment = EpochCommitment::from_terminal(2, terminal.compute_block_commitment());
    fixture.store_summary(commitment, fixture.genesis_epoch);
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(terminal.slot(), vec![terminal.compute_blkid()])
        .unwrap();

    let error = fixture.get_checkpoint_info(2).unwrap_err();

    assert_eq!(error.code(), INTERNAL_ERROR_CODE);
    let expected = BlockAncestryError::EpochTerminalMismatch {
        expected: commitment,
        actual: EpochCommitment::from_terminal(
            terminal.epoch(),
            terminal.compute_block_commitment(),
        ),
    };
    assert_eq!(error.message(), expected.to_string());
}

#[test]
fn test_nonterminal_checkpoint_summary_returns_internal_error() {
    let mut fixture = RpcFixture::new();
    let genesis = fixture.genesis_header();
    let nonterminal = fixture.append_block(&genesis);

    // A stored summary points at a block whose header does not carry the terminal flag.
    let commitment = EpochCommitment::from_terminal(1, nonterminal.compute_block_commitment());
    fixture.store_summary(commitment, fixture.genesis_epoch);
    fixture
        .storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(
            nonterminal.slot(),
            vec![nonterminal.compute_blkid()],
        )
        .unwrap();

    let error = fixture.get_checkpoint_info(1).unwrap_err();

    assert_eq!(error.code(), INTERNAL_ERROR_CODE);
    let expected = BlockAncestryError::MissingTerminalFlag(commitment);
    assert_eq!(error.message(), expected.to_string());
}

#[test]
fn test_css_rpc_resolves_finality_before_summary_application() {
    let fixture = RpcFixture::new();
    // L1 finality advances while the OL index and local summaries remain at genesis.
    let finalized = fixture.finalize_first_epoch();
    let provider = fixture.node_provider(FinalizedOLChainProvider::new(&fixture.storage));
    let module = fixture.public_module(NodeRole::CheckpointSync);
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
    fixture.store_summary(finalized, fixture.genesis_epoch);
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
        Some(fixture.genesis_epoch.to_block_commitment())
    );
    assert!(
        !module
            .method_names()
            .any(|name| name == "strata_getBlockBySlot")
    );
}

#[test]
fn test_css_rpc_reports_orphaned_client_state_as_retryable() {
    let fixture = RpcFixture::new();
    fixture.finalize_first_epoch();

    // Reorg out the L1 block containing the latest client-state row to model the
    // window before the client-state tracker catches up.
    fixture
        .storage
        .l1()
        .revert_canonical_chain(fixture.genesis_l1.height())
        .unwrap();

    let module = fixture.public_module(NodeRole::CheckpointSync);
    let error = test_runtime_handle()
        .block_on(
            module.call::<_, Option<RpcCheckpointInfo>>("strata_getCheckpointInfo", rpc_params![1]),
        )
        .expect_err("orphaned client state should be reported as retryable");
    let MethodsError::JsonRpc(error) = error else {
        panic!("test: expected a JSON-RPC error, got {error:?}");
    };

    assert_eq!(error.code(), FINALIZED_CLIENT_STATE_UNAVAILABLE_CODE);
}

#[test]
fn test_rpc_roles_select_index_or_finalized_epochs() {
    let fixture = RpcFixture::new();
    let finalized = fixture.finalize_first_epoch();
    fixture.store_summary(finalized, fixture.genesis_epoch);
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
        let module = fixture.public_module(role);
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
        let provider = fixture.node_provider(OLCanonicalChainProviderImpl::new(&fixture.storage));
        assert_eq!(
            test_runtime_handle()
                .block_on(provider.get_canonical_block_at(6))
                .unwrap(),
            Some(canonical.to_block_commitment())
        );
    }

    // CSS stays at the declared finality bound even when newer summaries and index entries exist.
    let module = fixture.public_module(NodeRole::CheckpointSync);
    let info: Option<RpcCheckpointInfo> = test_runtime_handle()
        .block_on(module.call("strata_getCheckpointInfo", rpc_params![2]))
        .unwrap();
    assert!(info.is_none());

    let submit = fixture.submit_module();
    assert_eq!(
        submit.method_names().collect::<Vec<_>>(),
        vec!["strata_submitTransaction"]
    );
}
