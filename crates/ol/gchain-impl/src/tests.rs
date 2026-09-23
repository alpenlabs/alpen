//! Exercises the OL stages over a small chain, through both kinds of link.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use strata_asm_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
};
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_gchain_executor::{
    ArtifactCache, GExecError, LinearExecutor, LinkOutcome, MemExecutorStore, PipelineBuilder,
    ProcContextImpl,
};
use strata_gchain_types::*;
use strata_identifiers::{
    Buf32, Buf64, Epoch, EpochCommitment, L1BlockCommitment, L1Height, OLBlockCommitment,
    OLBlockId, Slot,
};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1, SignedOLBlockHeaderV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{DaAccumulatingState, IndexerWrites, MemoryStateBaseLayer};
use strata_ol_state_types::IStateAccessor;
use strata_ol_state_types_v1::{IStateBatchApplicable, OLStateV1, WriteBatch};
use strata_ol_stf_v1::test_utils::{
    build_empty_chain, make_genesis_state, tamper_state_root, to_ol_block,
};
use strata_ol_stf_v1::{CompletedBlock, execute_block_batch_predrain};

use crate::*;

const SLOTS_PER_EPOCH: u64 = 2;

// ============================================================================
// In-memory stores
// ============================================================================

#[derive(Default)]
struct MemStateStore {
    states: Mutex<BTreeMap<OLStateNode, OLStateV1>>,
    terminal_headers: Mutex<Vec<OLBlockHeaderV1>>,
}

impl OLStateStore for MemStateStore {
    type State = MemoryStateBaseLayer;

    fn fetch_state(&self, node: &OLStateNode) -> Result<Option<Self::State>, ProcError> {
        Ok(self
            .states
            .lock()
            .unwrap()
            .get(node)
            .cloned()
            .map(MemoryStateBaseLayer::new))
    }

    fn derive_state(
        &self,
        base: &OLStateNode,
        target: &OLStateNode,
        diffs: &[&WriteBatch],
    ) -> Result<(), ProcError> {
        let mut states = self.states.lock().unwrap();
        let mut state = MemoryStateBaseLayer::new(states[base].clone());
        for diff in diffs {
            state
                .apply_write_batch((*diff).clone())
                .map_err(ProcError::custom)?;
        }
        states.insert(*target, state.into_inner());
        Ok(())
    }

    fn delete_state(&self, node: &OLStateNode) -> Result<(), ProcError> {
        self.states.lock().unwrap().remove(node);
        Ok(())
    }

    fn prune_states_before(&self, node: &OLStateNode) -> Result<(), ProcError> {
        self.states.lock().unwrap().retain(|n, _| n >= node);
        Ok(())
    }

    fn store_terminal_header(&self, header: &OLBlockHeaderV1) -> Result<(), ProcError> {
        self.terminal_headers.lock().unwrap().push(header.clone());
        Ok(())
    }
}

#[derive(Default)]
struct MemManifests(BTreeMap<L1Height, AsmManifest>);

impl L1ManifestProvider for MemManifests {
    fn fetch_manifest(&self, height: L1Height) -> Result<Option<AsmManifest>, ProcError> {
        Ok(self.0.get(&height).cloned())
    }
}

#[derive(Default)]
struct MemIndexStore {
    applied: Mutex<Vec<(OLLinkRef, IndexerWrites)>>,
}

impl OLIndexStore for MemIndexStore {
    fn apply_index_writes(
        &self,
        lref: &OLLinkRef,
        writes: &IndexerWrites,
    ) -> Result<(), ProcError> {
        self.applied.lock().unwrap().push((*lref, writes.clone()));
        Ok(())
    }

    fn revert_index_writes(&self, lref: &OLLinkRef) -> Result<(), ProcError> {
        self.applied.lock().unwrap().retain(|(l, _)| l != lref);
        Ok(())
    }
}

/// The chain graph as the provider sees it: every block of the fixture plus
/// whichever checkpoints a test adds.
#[derive(Default)]
struct MemChainStore {
    blocks: BTreeMap<OLBlockId, OLBlockV1>,
    summaries: BTreeMap<EpochCommitment, EpochSummary>,
    payloads: BTreeMap<EpochCommitment, CheckpointPayload>,
}

impl MemChainStore {
    fn from_chain(chain: &Chain) -> Self {
        let mut store = Self::default();
        for block in &chain.blocks {
            let block = to_ol_block(block);
            let blkid = *block.header().compute_block_commitment().blkid();
            store.blocks.insert(blkid, block);
        }
        store.add_summary(chain.genesis_summary());
        store
    }

    fn add_summary(&mut self, summary: EpochSummary) {
        self.summaries
            .insert(summary.get_epoch_commitment(), summary);
    }

    fn add_checkpoint(&mut self, link: &OLLink) {
        let OLLink::Checkpoint(ckpt) = link else {
            panic!("test: not a checkpoint link");
        };
        self.payloads.insert(
            ckpt.summary().get_epoch_commitment(),
            ckpt.payload().clone(),
        );
        self.add_summary(*ckpt.summary());
    }
}

impl OLChainStore for MemChainStore {
    fn fetch_block(&self, blkid: &OLBlockId) -> Result<Option<OLBlockV1>, ProviderError> {
        Ok(self.blocks.get(blkid).cloned())
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> Result<Option<OLBlockHeaderV1>, ProviderError> {
        Ok(self.blocks.get(blkid).map(|b| b.header().clone()))
    }

    fn fetch_blocks_at_slot(&self, slot: Slot) -> Result<Vec<OLBlockId>, ProviderError> {
        Ok(self
            .blocks
            .iter()
            .filter(|(_, b)| b.header().slot() == slot)
            .map(|(id, _)| *id)
            .collect())
    }

    fn fetch_epoch_summary(
        &self,
        epoch: &EpochCommitment,
    ) -> Result<Option<EpochSummary>, ProviderError> {
        Ok(self.summaries.get(epoch).cloned())
    }

    fn fetch_epoch_summaries_at(&self, epoch: Epoch) -> Result<Vec<EpochSummary>, ProviderError> {
        Ok(self
            .summaries
            .values()
            .filter(|s| s.epoch() == epoch)
            .cloned()
            .collect())
    }

    fn fetch_checkpoint_payload(
        &self,
        epoch: &EpochCommitment,
    ) -> Result<Option<CheckpointPayload>, ProviderError> {
        Ok(self.payloads.get(epoch).cloned())
    }
}

// ============================================================================
// Chain fixture
// ============================================================================

/// A chain of empty blocks with the state after genesis, which is where every
/// path in these tests starts.
struct Chain {
    blocks: Vec<CompletedBlock>,
    post_genesis: OLStateV1,
}

impl Chain {
    fn build(num_blocks: usize) -> Self {
        let mut state = make_genesis_state();
        let blocks = build_empty_chain(&mut state, num_blocks, SLOTS_PER_EPOCH).expect("chain");

        // Both runs start from the same genesis state, so the genesis blocks
        // are identical and the one-block run's post-state is what the
        // longer chain's first block builds on.
        let mut genesis_only = make_genesis_state();
        build_empty_chain(&mut genesis_only, 1, SLOTS_PER_EPOCH).expect("genesis");

        Self {
            blocks,
            post_genesis: genesis_only.into_inner(),
        }
    }

    fn header(&self, slot: usize) -> &OLBlockHeaderV1 {
        self.blocks[slot].header()
    }

    fn node(&self, slot: usize) -> OLStateNode {
        OLStateNode::from_header(self.header(slot))
    }

    fn commitment(&self, slot: usize) -> OLBlockCommitment {
        self.header(slot).compute_block_commitment()
    }

    fn block_link(&self, slot: usize) -> (OLLinkRef, OLLink) {
        let block = to_ol_block(&self.blocks[slot]);
        let parent = Some(self.header(slot - 1).clone());
        (
            self.commitment(slot).into(),
            OLLink::Block(OLBlockLink::new(block, parent)),
        )
    }

    fn endpoints(&self, slot: usize) -> LinkEndpoints<OLChainSpec> {
        LinkEndpoints::new(self.node(slot - 1), self.node(slot))
    }

    /// Builds the checkpoint link for an epoch, along with the manifests it
    /// needs.  Epoch `e` spans the `SLOTS_PER_EPOCH` slots ending at slot
    /// `e * SLOTS_PER_EPOCH` and departs from the previous epoch's terminal.
    fn checkpoint_for_epoch(&self, epoch: u32) -> (OLLinkRef, OLLink, MemManifests) {
        let terminal = epoch as usize * SLOTS_PER_EPOCH as usize;
        let prev_terminal = terminal - SLOTS_PER_EPOCH as usize;
        let blocks: Vec<OLBlockV1> = (prev_terminal + 1..=terminal)
            .map(|s| to_ol_block(&self.blocks[s]))
            .collect();

        let mut da = DaAccumulatingState::new(self.state_after(prev_terminal));
        let logs = execute_block_batch_predrain(
            &mut da,
            &blocks,
            self.header(prev_terminal),
            &OLRuntimeParams::test_default(),
        )
        .expect("replay epoch");
        let diff = da
            .take_completed_epoch_da_blob()
            .expect("finalize DA")
            .expect("DA blob");

        let term_header = self.header(terminal);
        let post_epoch = self.state_after(terminal);
        let l1_height = post_epoch.last_l1_height();
        let l1_blkid = *post_epoch.last_l1_blkid();

        let tip = CheckpointTip::new(epoch, l1_height, self.commitment(terminal));
        let complement = TerminalHeaderComplement::new(
            term_header.timestamp(),
            *term_header.parent_blkid(),
            *term_header.body_root(),
            *term_header.logs_root(),
        );
        let sidecar = CheckpointSidecar::new(diff, logs, complement).expect("sidecar");
        let payload = CheckpointPayload::new(tip, sidecar, Vec::new()).expect("payload");
        let summary = EpochSummary::new(
            epoch,
            self.commitment(terminal),
            self.commitment(prev_terminal),
            L1BlockCommitment::new(l1_height, l1_blkid),
            *term_header.state_root(),
        );

        let manifests = blocks
            .iter()
            .filter_map(|b| b.body().manifests())
            .flat_map(|c| c.manifests().iter().cloned())
            .map(|m| (m.height(), m))
            .collect();

        (
            summary.get_epoch_commitment().into(),
            OLLink::Checkpoint(OLCheckpointLink::new(summary, payload)),
            MemManifests(manifests),
        )
    }

    /// The summary of the genesis epoch, which a checkpoint for epoch 1
    /// departs from.
    fn genesis_summary(&self) -> EpochSummary {
        let post_genesis = self.state_after(0);
        let l1_height = post_genesis.last_l1_height();
        let l1_blkid = *post_genesis.last_l1_blkid();
        EpochSummary::new(
            0,
            self.commitment(0),
            OLBlockCommitment::null(),
            L1BlockCommitment::new(l1_height, l1_blkid),
            *self.header(0).state_root(),
        )
    }

    /// The state after a slot, reached by executing directly.
    fn state_after(&self, slot: usize) -> MemoryStateBaseLayer {
        let mut state = make_genesis_state();
        build_empty_chain(&mut state, slot + 1, SLOTS_PER_EPOCH).expect("chain");
        state
    }
}

// ============================================================================
// Harness
// ============================================================================

fn proc_id(s: &str) -> ProcId {
    ProcId::from_str(s).unwrap()
}

/// Drives stages over links by hand, standing in for the executor.
struct Harness {
    chain: Chain,
    state_store: Arc<MemStateStore>,
    cache: ArtifactCache<OLChainSpec>,
    path: LinkPath<OLChainSpec>,
}

impl Harness {
    fn new(chain: Chain) -> Self {
        let state_store = Arc::new(MemStateStore::default());
        state_store
            .states
            .lock()
            .unwrap()
            .insert(chain.node(0), chain.post_genesis.clone());
        Self {
            path: LinkPath::new_at(chain.node(0)),
            chain,
            state_store,
            cache: ArtifactCache::new(),
        }
    }

    fn exec_proc(&self, manifests: MemManifests) -> OLExecProc<MemStateStore, MemManifests> {
        OLExecProc::new(
            OLRuntimeParams::test_default(),
            Arc::clone(&self.state_store),
            Arc::new(manifests),
        )
    }

    fn index_proc(
        &self,
        manifests: MemManifests,
    ) -> OLIndexProc<MemStateStore, MemManifests, MemIndexStore> {
        OLIndexProc::new(
            proc_id("exec"),
            OLRuntimeParams::test_default(),
            Arc::clone(&self.state_store),
            Arc::new(manifests),
            MemIndexStore::default(),
        )
    }

    /// Runs a stage on a link at the end of the current path and caches its
    /// artifact under the ID.
    fn run<P: GChainProc<Spec = OLChainSpec>>(
        &mut self,
        proc: &P,
        id: &str,
        lref: &OLLinkRef,
        link: &OLLink,
    ) -> Result<Arc<P::Artifact>, ProcError> {
        let ctx = ProcContextImpl::<P>::new(&self.cache, &self.path, *lref, proc_id(id));
        let artifact = Arc::new(proc.process_link(lref, link, &ctx)?);
        let erased = Arc::clone(&artifact) as Arc<dyn DynProcArtifact>;
        self.cache.insert_artifact(*lref, proc_id(id), erased);
        Ok(artifact)
    }

    fn extend_path(&mut self, lref: OLLinkRef, endpoints: &LinkEndpoints<OLChainSpec>) {
        assert!(self.path.try_push_link(lref, endpoints));
    }

    fn cached<A: ProcArtifact>(&self, id: &str) -> Vec<Arc<A>> {
        self.path
            .links()
            .iter()
            .map(|l| self.cache.get_artifact::<A>(l, proc_id(id)).unwrap())
            .collect()
    }

    fn stored_root(&self, node: &OLStateNode) -> Option<Buf32> {
        self.state_store
            .fetch_state(node)
            .unwrap()
            .map(|s| s.compute_state_root().unwrap())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_exec_block_path_reaches_executed_state() {
    let mut h = Harness::new(Chain::build(5));
    let exec = h.exec_proc(MemManifests::default());

    for slot in 1..5 {
        let (lref, link) = h.chain.block_link(slot);
        let artifact = h.run(&exec, "exec", &lref, &link).expect("process");
        let output = artifact.output().expect("valid block");
        assert_eq!(output.header(), h.chain.header(slot));
        assert_eq!(
            output.target_node(),
            h.chain.node(slot),
            "artifact must land on the block's node"
        );
        h.extend_path(lref, &h.chain.endpoints(slot));
    }

    exec.commit_outputs(&h.path, &h.cached("exec"))
        .expect("commit");

    assert_eq!(
        h.stored_root(&h.chain.node(4)),
        Some(*h.chain.header(4).state_root()),
        "committed state must match direct execution"
    );
    assert!(
        h.stored_root(&h.chain.node(0)).is_some(),
        "base state must survive a commit so the path can be rolled back"
    );
}

#[test]
fn test_exec_rejects_block_with_wrong_state_root() {
    let mut h = Harness::new(Chain::build(2));
    let exec = h.exec_proc(MemManifests::default());

    let tampered = tamper_state_root(h.chain.header(1), Buf32::from([0xab; 32]));
    let block = OLBlockV1::new(
        SignedOLBlockHeaderV1::new(tampered.clone(), Buf64::zero()),
        h.blocks_body(1),
    );
    let lref = tampered.compute_block_commitment().into();
    let link = OLLink::Block(OLBlockLink::new(block, Some(h.chain.header(0).clone())));

    let artifact = h.run(&exec, "exec", &lref, &link).expect("process");
    assert!(
        !ProcArtifact::is_link_valid(&*artifact),
        "tampered block must be rejected, got {artifact:?}"
    );
    assert!(artifact.invalid_reason().is_some());
}

impl Harness {
    fn blocks_body(&self, slot: usize) -> strata_ol_chain_types_v1::OLBlockBodyV1 {
        self.chain.blocks[slot].body().clone()
    }
}

#[test]
fn test_exec_checkpoint_reaches_block_path_state() {
    let mut h = Harness::new(Chain::build(3));
    let (lref, link, manifests) = h.chain.checkpoint_for_epoch(1);
    let exec = h.exec_proc(manifests);

    let artifact = h.run(&exec, "exec", &lref, &link).expect("process");
    let output = artifact.output().unwrap_or_else(|| {
        panic!(
            "checkpoint must be valid, got {:?}",
            artifact.invalid_reason()
        )
    });
    assert_eq!(
        output.header(),
        h.chain.header(2),
        "reconstructed terminal header must match the real one"
    );

    h.extend_path(lref, &LinkEndpoints::new(h.chain.node(0), h.chain.node(2)));
    exec.commit_outputs(&h.path, &h.cached("exec"))
        .expect("commit");

    assert_eq!(
        h.stored_root(&h.chain.node(2)),
        Some(*h.chain.header(2).state_root())
    );
    assert_eq!(
        h.state_store.terminal_headers.lock().unwrap().as_slice(),
        &[h.chain.header(2).clone()],
        "checkpoint commit must persist the reconstructed header"
    );
}

#[test]
fn test_checkpoint_after_uncommitted_blocks_uses_path_state() {
    // Blocks of epoch 1 are processed but not committed, then the checkpoint
    // for epoch 2 departs from the uncommitted epoch 1 terminal.
    let mut h = Harness::new(Chain::build(5));
    let exec = h.exec_proc(MemManifests::default());

    for slot in 1..=2 {
        let (lref, link) = h.chain.block_link(slot);
        h.run(&exec, "exec", &lref, &link).expect("process");
        h.extend_path(lref, &h.chain.endpoints(slot));
    }

    // The epoch 2 checkpoint is built the same way as epoch 1's, so reuse the
    // builder by shifting which blocks it looks at.
    let (lref, link, manifests) = h.chain.checkpoint_for_epoch(2);
    let exec = h.exec_proc(manifests);
    let artifact = h.run(&exec, "exec", &lref, &link).expect("process");
    assert_eq!(
        artifact.output().map(|o| o.header()),
        Some(h.chain.header(4)),
        "got {:?}",
        artifact.invalid_reason()
    );
}

#[test]
fn test_index_stage_follows_exec_and_refuses_rejected_links() {
    let mut h = Harness::new(Chain::build(3));
    let (lref, link, manifests) = h.chain.checkpoint_for_epoch(1);
    let exec = h.exec_proc(MemManifests(manifests.0.clone()));
    let index = h.index_proc(manifests);

    h.run(&exec, "exec", &lref, &link).expect("exec");
    let artifact = h.run(&index, "index", &lref, &link).expect("index");
    assert!(
        !artifact.writes().l1_block_records().is_empty(),
        "terminal manifests must produce L1 block record writes"
    );

    // The executor never runs the index stage on a link exec rejected, so
    // being asked to is a missing dep rather than something to index.
    let bad_lref: OLLinkRef = OLBlockCommitment::new(9, Default::default()).into();
    h.cache.insert_artifact(
        bad_lref,
        proc_id("exec"),
        Arc::new(OLExecArtifact::Invalid("nope".into())),
    );
    let err = h.run(&index, "index", &bad_lref, &link).unwrap_err();
    assert!(matches!(err, ProcError::MissingDep(id) if id == proc_id("exec")));
}

#[test]
fn test_artifacts_roundtrip_through_buf() {
    let mut h = Harness::new(Chain::build(3));
    let (lref, link, manifests) = h.chain.checkpoint_for_epoch(1);
    let exec = h.exec_proc(MemManifests(manifests.0.clone()));
    let index = h.index_proc(manifests);

    let ea = h.run(&exec, "exec", &lref, &link).expect("exec");
    let decoded = OLExecArtifact::from_buf(&ea.to_buf().unwrap()).unwrap();
    assert_eq!(
        decoded.output().unwrap().header(),
        ea.output().unwrap().header()
    );
    assert_eq!(
        decoded.output().unwrap().logs(),
        ea.output().unwrap().logs()
    );

    let ia = h.run(&index, "index", &lref, &link).expect("index");
    let decoded = OLIndexArtifact::from_buf(&ia.to_buf().unwrap()).unwrap();
    assert_eq!(
        decoded.writes().l1_block_records().len(),
        ia.writes().l1_block_records().len()
    );

    let invalid = OLExecArtifact::Invalid("reason".into());
    let decoded = OLExecArtifact::from_buf(&invalid.to_buf().unwrap()).unwrap();
    assert_eq!(decoded.invalid_reason(), Some("reason"));
}

/// Runs both OL stages under the real executor: blocks of epoch 1 are
/// processed and committed, the checkpoint for epoch 2 departs from that
/// committed node, and the commit is then rolled back.
#[test]
fn test_linear_executor_drives_ol_stages() {
    let chain = Chain::build(5);
    let state_store = Arc::new(MemStateStore::default());
    state_store
        .states
        .lock()
        .unwrap()
        .insert(chain.node(0), chain.post_genesis.clone());

    let mut chain_store = MemChainStore::from_chain(&chain);
    let mut manifests = MemManifests::default();
    let mut ckpt_refs = Vec::new();
    for epoch in [1, 2] {
        let (lref, link, epoch_manifests) = chain.checkpoint_for_epoch(epoch);
        chain_store.add_checkpoint(&link);
        manifests.0.extend(epoch_manifests.0);
        ckpt_refs.push(lref);
    }
    let manifests = Arc::new(manifests);

    let exec_proc = OLExecProc::new(
        OLRuntimeParams::test_default(),
        Arc::clone(&state_store),
        Arc::clone(&manifests),
    );
    let index_proc = OLIndexProc::new(
        proc_id("exec"),
        OLRuntimeParams::test_default(),
        Arc::clone(&state_store),
        manifests,
        MemIndexStore::default(),
    );
    let index_deps = index_proc.deps();
    let pipeline = PipelineBuilder::new()
        .add_stage(
            proc_id("exec"),
            exec_proc,
            ProcDeps::new(Vec::new(), vec![proc_id("exec")]),
        )
        .expect("add exec")
        .add_stage(proc_id("index"), index_proc, index_deps)
        .expect("add index")
        .build();

    let (mut exec, report) = LinearExecutor::open(
        pipeline,
        Arc::new(OLChainProvider::new(chain_store)),
        Arc::new(MemExecutorStore::new()),
        chain.node(0),
    )
    .expect("open");
    assert_eq!(report.initialized(), &[proc_id("exec"), proc_id("index")]);

    let block_refs: Vec<OLLinkRef> = (1..=2).map(|s| chain.commitment(s).into()).collect();
    for lref in &block_refs {
        assert_eq!(
            exec.process_link(lref).expect("process block"),
            LinkOutcome::Accepted
        );
    }
    assert!(
        exec.get_artifact::<OLIndexArtifact>(&block_refs[1], proc_id("index"))
            .is_some(),
        "index stage must have run after exec"
    );

    exec.commit_through(&block_refs[1]).expect("commit blocks");
    assert_eq!(exec.committed_node(), &chain.node(2));
    assert_eq!(
        state_store
            .fetch_state(&chain.node(2))
            .unwrap()
            .map(|s| s.compute_state_root().unwrap()),
        Some(*chain.header(2).state_root()),
        "committed state must match direct execution"
    );

    // The epoch 2 checkpoint departs from the committed node.
    assert_eq!(
        exec.process_link(&ckpt_refs[1])
            .expect("process checkpoint"),
        LinkOutcome::Accepted
    );
    exec.commit_through(&ckpt_refs[1])
        .expect("commit checkpoint");
    assert_eq!(exec.committed_node(), &chain.node(4));
    assert_eq!(
        state_store.terminal_headers.lock().unwrap().as_slice(),
        &[chain.header(4).clone()],
        "checkpoint commit must persist the reconstructed header"
    );

    exec.uncommit_to(&chain.node(2)).expect("uncommit");
    assert_eq!(exec.committed_node(), &chain.node(2));
    assert!(
        state_store.fetch_state(&chain.node(4)).unwrap().is_none(),
        "uncommit must drop the derived state"
    );
    assert!(
        exec.is_processed(&ckpt_refs[1]),
        "the undone link stays processed"
    );

    // The epoch 1 checkpoint converges on the committed node from below, so
    // it isn't reachable until the blocks are uncommitted too.
    let err = exec.process_link(&ckpt_refs[0]).unwrap_err();
    assert!(matches!(err, GExecError::OriginUnreachable(_)));
    exec.uncommit_to(&chain.node(0))
        .expect("uncommit to genesis");
    assert_eq!(
        exec.process_link(&ckpt_refs[0])
            .expect("process checkpoint"),
        LinkOutcome::Accepted
    );
}
