use std::{collections::BTreeMap, sync::Arc};

use jsonrpc_core::server::MethodsError;
use jsonrpsee::{RpcModule, core as jsonrpc_core, rpc_params};
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{ClientState, L1Status};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_tests::asm_tests::make_test_asm_state;
use strata_db_types::ol_state_index::IndexingWrites;
use strata_identifiers::{AccountId, Buf32, EpochCommitment, L1BlockCommitment};
use strata_ol_canonical_chain_provider::{
    FinalizedOLChainProvider, OLCanonicalChainProvider, OLCanonicalChainProviderImpl,
    test_utils::{build_checkpoint_payload, finalize_epoch, record_l1_observation},
};
use strata_ol_rpc_types::{OLRpcProvider, RpcCheckpointInfo};
use strata_status::StatusChannel;
use strata_storage::{NodeStorage, create_node_storage, test_runtime_handle};

use super::{
    RpcDeps, build_public_rpc_module, build_submit_rpc_module,
    errors::FINALIZED_CLIENT_STATE_UNAVAILABLE_CODE, provider::NodeRpcProvider,
};
use crate::run_context::NodeRole;

struct RpcFixture {
    /// Node storage the RPC modules and providers read.
    storage: Arc<NodeStorage>,

    /// Genesis epoch.
    genesis_epoch: EpochCommitment,

    /// L1 block OL genesis anchors to.
    genesis_l1: L1BlockCommitment,

    /// Shared by every module and provider, as on a node.
    status_channel: Arc<StatusChannel>,
}

impl RpcFixture {
    /// Creates a fixture whose storage holds the genesis epoch's summary, its last block in
    /// the canonical block index, and `genesis_l1` as the canonical L1 chain.
    fn new() -> Self {
        let genesis_l1 = L1BlockCommitment::new(100, Buf32::from([100; 32]).into());
        let fixture = Self {
            storage: Arc::new(
                create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap(),
            ),
            genesis_epoch: EpochCommitment::new(0, 0, Buf32::from([1; 32]).into()),
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
