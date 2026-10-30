use std::{collections::BTreeMap, sync::Arc};

use jsonrpsee::rpc_params;
use strata_asm_checkpoint_types::{
    CheckpointPayload, CheckpointTip, test_utils::create_test_checkpoint_payload,
};
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint, L1Status};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_tests::asm_tests::make_test_asm_state;
use strata_db_types::ol_state_index::IndexingWrites;
use strata_identifiers::{AccountId, Buf32, EpochCommitment, L1BlockCommitment, RBuf32};
use strata_ol_canonical_chain_provider::{FinalizedOLChainProvider, OLCanonicalChainProviderImpl};
use strata_ol_rpc_types::{OLRpcProvider, RpcCheckpointInfo};
use strata_status::StatusChannel;
use strata_storage::{NodeStorage, create_node_storage, test_runtime_handle};

use super::{RpcDeps, build_public_rpc_module, build_submit_rpc_module, provider::NodeRpcProvider};
use crate::run_context::NodeRole;

struct RpcFixture {
    storage: Arc<NodeStorage>,
    genesis: EpochCommitment,
    genesis_l1: L1BlockCommitment,
}

impl RpcFixture {
    fn new() -> Self {
        let fixture = Self {
            storage: Arc::new(
                create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap(),
            ),
            genesis: EpochCommitment::new(0, 0, Buf32::from([1; 32]).into()),
            genesis_l1: L1BlockCommitment::new(100, Buf32::from([100; 32]).into()),
        };
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
