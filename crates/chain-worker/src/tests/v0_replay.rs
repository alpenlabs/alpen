//! Checkpoint-sync replay of a network launched on 0.3.0, across its switch
//! from V0 to V1.
//!
//! `testdata/v0_epochs.json` holds a V0 chain that a scratch generator built on
//! `releases/0.3.0` from MN0-shaped params, with that release's STF, DA
//! accumulator and checkpoint payload rules:
//!
//! 1. no manifests, so the next epoch starts from a state still at the genesis L1 anchor;
//! 2. a deposit to the EE account, two deposits V0 sweeps to limbo (a malformed destination and an
//!    unknown account serial), and logs of the types V0 ignores (2, 11, 12 and 13);
//! 3. no manifests, after a non-genesis manifest;
//! 4. an EE update that consumes the deposit and withdraws through the bridge, a checkpoint tip
//!    update, and a second deposit;
//! 5. an EE update that consumes the second deposit, then an EE key update in the terminal drain;
//! 6. an EE update under the new key, ending at a checkpoint predicate enactment, the last V0
//!    epoch.
//!
//! Each epoch records what its checkpoint carries, its L1 manifests, and the
//! terminal block ID, state root and limbo balance 0.3.0 computed. The file's
//! `source` field says how the generator produced them. The epochs after the
//! enactment are built here with the V1 rules, as the promoted sequencer
//! builds them.

use serde::Deserialize;
use strata_acct_types::{AccountId, BitcoinAmount};
use strata_asm_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, OLLog as CheckpointOLLog,
    TerminalHeaderComplement,
};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_identifiers::{
    AccountSerial, Buf32, EpochCommitment, L1BlockId, OLBlockCommitment, OLBlockId, SubjectId,
    WtxidsRoot,
};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_genesis::build_genesis_artifacts;
use strata_ol_params::OLParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};
use strata_ol_state_types::{
    IAccountState, ISnarkAccountState, IStateAccessor, OLSpecId, OLSpecVersions,
};
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, execute_and_complete_block,
    execute_block_batch_predrain,
};
use strata_ol_stf_v1::test_utils::{
    make_checkpoint_predicate_enactment_manifest, make_deposit_manifest_for_account, to_ol_block,
};
use strata_predicate::{PredicateKey, PredicateTypeId};

use super::apply_checkpoint::MockChainWorkerContext;
use crate::{
    WorkerError, WorkerResult,
    state::{AppliedEpochArtifacts, apply_checkpoint_epoch},
};

const FIXTURE_JSON: &str = include_str!("testdata/v0_epochs.json");

/// Spacing of the block timestamps the V1 epochs here use, as in the fixture.
const BLOCK_TIMESTAMP_STEP: u64 = 5_000;

#[derive(Deserialize)]
struct Fixture {
    params: OLParams,
    ee_account: String,
    ee_key_update_epoch: u32,
    rotated_ee_key: FixtureKey,
    genesis: FixtureGenesis,
    epochs: Vec<FixtureEpoch>,
}

#[derive(Deserialize)]
struct FixtureKey {
    type_id: u8,
    condition: String,
}

#[derive(Deserialize)]
struct FixtureGenesis {
    state_root: String,
    blkid: String,
}

#[derive(Deserialize)]
struct FixtureEpoch {
    epoch: u32,
    l1_height: u32,
    limbo_sat: u64,
    da_payload: String,
    ol_logs: Vec<FixtureLog>,
    manifests: Vec<FixtureManifest>,
    terminal: FixtureTerminal,
}

#[derive(Deserialize)]
struct FixtureLog {
    serial: u32,
    payload: String,
}

#[derive(Deserialize)]
struct FixtureManifest {
    height: u32,
    blkid: String,
    wtxids_root: String,
    logs: Vec<String>,
}

#[derive(Deserialize)]
struct FixtureTerminal {
    timestamp: u64,
    slot: u64,
    parent_blkid: String,
    body_root: String,
    logs_root: String,
    state_root: String,
    blkid: String,
}

fn bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex).expect("fixture hex")
}

fn buf32(hex: &str) -> Buf32 {
    let bytes: [u8; 32] = bytes(hex).try_into().expect("fixture value is 32 bytes");
    Buf32::from(bytes)
}

impl Fixture {
    fn load() -> Self {
        serde_json::from_str(FIXTURE_JSON).expect("fixture parses")
    }

    fn ee_account(&self) -> AccountId {
        let bytes: [u8; 32] = bytes(&self.ee_account).try_into().expect("account ID");
        AccountId::from(bytes)
    }

    fn rotated_ee_key(&self) -> PredicateKey {
        let type_id =
            PredicateTypeId::try_from(self.rotated_ee_key.type_id).expect("known predicate type");
        PredicateKey::try_new(type_id, bytes(&self.rotated_ee_key.condition))
            .expect("predicate condition fits")
    }

    /// Returns the last V0 epoch, whose last manifest carries the enactment.
    fn last_v0_epoch(&self) -> &FixtureEpoch {
        self.epochs.last().expect("fixture has epochs")
    }
}

impl FixtureEpoch {
    fn terminal_commitment(&self) -> OLBlockCommitment {
        OLBlockCommitment::new(
            self.terminal.slot,
            OLBlockId::from(buf32(&self.terminal.blkid)),
        )
    }

    fn epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::from_terminal(self.epoch, self.terminal_commitment())
    }

    fn checkpoint_payload(&self) -> CheckpointPayload {
        let complement = TerminalHeaderComplement::new(
            self.terminal.timestamp,
            OLBlockId::from(buf32(&self.terminal.parent_blkid)),
            buf32(&self.terminal.body_root),
            buf32(&self.terminal.logs_root),
        );
        let ol_logs = self
            .ol_logs
            .iter()
            .map(|log| CheckpointOLLog::new(AccountSerial::from(log.serial), bytes(&log.payload)))
            .collect();
        let sidecar = CheckpointSidecar::new(bytes(&self.da_payload), ol_logs, complement)
            .expect("checkpoint sidecar");
        let tip = CheckpointTip::new(self.epoch, self.l1_height, self.terminal_commitment());
        CheckpointPayload::new(tip, sidecar, Vec::new()).expect("checkpoint payload")
    }

    fn manifests(&self) -> impl Iterator<Item = AsmManifest> + '_ {
        self.manifests.iter().map(|manifest| {
            let logs = manifest
                .logs
                .iter()
                .map(|log| AsmLogEntry::from_raw(bytes(log)).expect("ASM log"))
                .collect();
            AsmManifest::new(
                manifest.height,
                L1BlockId::from(buf32(&manifest.blkid)),
                WtxidsRoot::from(buf32(&manifest.wtxids_root)),
                logs,
            )
            .expect("manifest")
        })
    }
}

/// A checkpoint-sync node: it stores what [`apply_checkpoint_epoch`] reads,
/// and the result of every epoch it applies.
struct Node {
    ctx: MockChainWorkerContext,
}

impl Node {
    /// Starts a node from the V0 genesis of `params`, which knows the L1
    /// manifests and checkpoints of `epochs`.
    fn new(params: &OLParams, epochs: &[FixtureEpoch]) -> Self {
        let genesis = build_genesis_artifacts(params).expect("V0 genesis builds");
        let mut ctx = MockChainWorkerContext::new();
        ctx.runtime_params = params.runtime_params();
        ctx.genesis_l1_block = params.genesis_l1_block();
        ctx.epoch_summaries.insert(0, vec![genesis.epoch_summary]);
        ctx.ol_states.insert(genesis.commitment, genesis.ol_state);
        for epoch in epochs {
            for manifest in epoch.manifests() {
                ctx.manifests.insert(manifest.height(), manifest);
            }
            ctx.checkpoint_payloads
                .insert(epoch.epoch_commitment(), epoch.checkpoint_payload());
        }
        Self { ctx }
    }

    /// Replays `epochs` in order, asserting each reproduces what 0.3.0
    /// recorded.
    fn replay_v0(&mut self, epochs: &[FixtureEpoch]) -> AppliedEpochArtifacts {
        let mut last = None;
        for epoch in epochs {
            let artifacts = self
                .apply(epoch.epoch_commitment())
                .unwrap_or_else(|err| panic!("epoch {} replays: {err}", epoch.epoch));
            assert_eq!(
                *artifacts.summary.final_state(),
                buf32(&epoch.terminal.state_root),
                "epoch {} state root",
                epoch.epoch
            );
            assert_eq!(
                artifacts.terminal_header.compute_blkid(),
                OLBlockId::from(buf32(&epoch.terminal.blkid)),
                "epoch {} terminal block ID",
                epoch.epoch
            );
            assert_eq!(
                artifacts.new_state.spec_versions(),
                OLSpecVersions::uniform(OLSpecId::V0),
                "epoch {} stays V0",
                epoch.epoch
            );
            let state = MemoryStateBaseLayer::from_container(artifacts.new_state.clone());
            assert_eq!(
                state.limbo_funds().to_sat(),
                epoch.limbo_sat,
                "epoch {} limbo balance",
                epoch.epoch
            );
            last = Some(artifacts);
        }
        last.expect("replayed at least one epoch")
    }

    /// Applies the checkpoint of `epoch` and stores its state and summary.
    fn apply(&mut self, epoch: EpochCommitment) -> WorkerResult<AppliedEpochArtifacts> {
        let artifacts = apply_checkpoint_epoch(&self.ctx, epoch)?;
        self.ctx
            .epoch_summaries
            .insert(epoch.epoch(), vec![artifacts.summary]);
        self.ctx
            .ol_states
            .insert(epoch.to_block_commitment(), artifacts.new_state.clone());
        Ok(artifacts)
    }

    /// Adds an epoch built after the fixture's, with its manifests.
    fn add_epoch(&mut self, built: &BuiltV1Epoch) {
        for manifest in &built.manifests {
            self.ctx
                .manifests
                .insert(manifest.height(), manifest.clone());
        }
        self.ctx
            .checkpoint_payloads
            .insert(built.epoch_commitment, built.checkpoint_payload.clone());
    }

    /// Returns a node restarted with only what this node stored for the
    /// epochs up to `epoch`: the restart keeps no state in memory.
    fn restarted_at(&self, epoch: u32) -> Self {
        let mut ctx = MockChainWorkerContext::new();
        ctx.runtime_params = self.ctx.runtime_params;
        ctx.genesis_l1_block = self.ctx.genesis_l1_block;
        let summary = self.ctx.epoch_summaries[&epoch][0];
        ctx.epoch_summaries.insert(epoch, vec![summary]);
        ctx.ol_states.insert(
            *summary.terminal(),
            self.ctx.ol_states[summary.terminal()].clone(),
        );
        ctx.manifests = self.ctx.manifests.clone();
        ctx.checkpoint_payloads = self.ctx.checkpoint_payloads.clone();
        Self { ctx }
    }
}

/// A V1 epoch built with block execution, with what its checkpoint carries.
struct BuiltV1Epoch {
    epoch_commitment: EpochCommitment,
    manifests: Vec<AsmManifest>,
    checkpoint_payload: CheckpointPayload,
    terminal_header: OLBlockHeaderV1,
    post_state: OLStateContainer,
}

/// Builds a V1 epoch on `parent_state` with one empty block and a terminal
/// carrying `manifests`, and derives its checkpoint payload the way the
/// checkpoint builder does.
fn build_v1_epoch(
    params: &OLParams,
    parent_state: &OLStateContainer,
    parent_header: &OLBlockHeaderV1,
    manifests: Vec<AsmManifest>,
) -> BuiltV1Epoch {
    let runtime_params = params.runtime_params();
    let pre_epoch_state = MemoryStateBaseLayer::from_container(parent_state.clone());
    let mut state = pre_epoch_state.clone();
    let epoch = parent_header.epoch() + 1;

    let mut blocks: Vec<OLBlockV1> = Vec::new();
    let mut parent = parent_header.clone();
    let components = [
        BlockComponents::new_empty(),
        BlockComponents::new_manifests(manifests.clone()).as_terminal(),
    ];
    for components in components {
        let slot = parent.slot() + 1;
        let info = BlockInfo::new(parent.timestamp() + BLOCK_TIMESTAMP_STEP, slot, epoch);
        let block = execute_and_complete_block(
            OLSpecId::V1,
            &mut state,
            BlockContext::new(&info, Some(&parent)),
            components,
            &runtime_params,
        )
        .expect("V1 block executes");
        parent = block.header().clone();
        blocks.push(to_ol_block(&block));
    }
    let terminal_header = parent;

    let mut da = DaAccumulatingState::new(pre_epoch_state);
    let logs = execute_block_batch_predrain(
        OLSpecId::V1,
        &mut da,
        &blocks,
        parent_header,
        &runtime_params,
    )
    .expect("predrain replay");
    let da_blob = da
        .take_completed_epoch_da_blob()
        .expect("finalize DA")
        .expect("DA blob");
    let ol_logs = logs
        .into_iter()
        .map(|log| CheckpointOLLog::new(log.account_serial(), log.payload().to_vec()))
        .collect();
    let complement = TerminalHeaderComplement::new(
        terminal_header.timestamp(),
        *terminal_header.parent_blkid(),
        *terminal_header.body_root(),
        *terminal_header.logs_root(),
    );
    let sidecar = CheckpointSidecar::new(da_blob, ol_logs, complement).expect("checkpoint sidecar");
    let terminal = terminal_header.compute_block_commitment();
    let tip = CheckpointTip::new(epoch, state.last_l1_height(), terminal);
    let checkpoint_payload =
        CheckpointPayload::new(tip, sidecar, Vec::new()).expect("checkpoint payload");

    BuiltV1Epoch {
        epoch_commitment: EpochCommitment::from_terminal(epoch, terminal),
        manifests,
        checkpoint_payload,
        terminal_header,
        post_state: state.into_container(),
    }
}

/// Builds the first V1 epoch on the last V0 epoch: one deposit to the EE
/// account in the manifest after the enactment.
fn build_first_v1_epoch(fixture: &Fixture, last_v0: &AppliedEpochArtifacts) -> BuiltV1Epoch {
    let state = MemoryStateBaseLayer::from_container(last_v0.new_state.clone());
    let ee_serial = state
        .get_account_state(fixture.ee_account())
        .expect("read EE account")
        .expect("EE account exists")
        .serial();
    let deposit = make_deposit_manifest_for_account(
        state.last_l1_height() + 1,
        0xd4,
        ee_serial,
        SubjectId::from([0xd4; 32]),
        BitcoinAmount::try_from(200_000_000).expect("amount fits the money supply"),
    );
    build_v1_epoch(
        &fixture.params,
        &last_v0.new_state,
        &last_v0.terminal_header,
        vec![deposit],
    )
}

#[test]
fn test_v0_epochs_replay_to_recorded_roots_and_block_ids() {
    let fixture = Fixture::load();
    let genesis = build_genesis_artifacts(&fixture.params).expect("V0 genesis builds");
    assert_eq!(
        *genesis.ol_block.header().state_root(),
        buf32(&fixture.genesis.state_root)
    );
    assert_eq!(
        *genesis.commitment.blkid(),
        OLBlockId::from(buf32(&fixture.genesis.blkid))
    );

    let mut node = Node::new(&fixture.params, &fixture.epochs);
    let last_v0 = node.replay_v0(&fixture.epochs);

    // V0 roots are bare chainstate roots.
    assert_eq!(
        *last_v0.summary.final_state(),
        last_v0.new_state.chainstate().compute_chainstate_root()
    );
}

/// The deposits with a malformed destination and an unknown account serial
/// went to limbo, as under V0, and the logs V0 ignores changed nothing
/// else: the epoch reproduces the root 0.3.0 recorded.
#[test]
fn test_v0_limbo_deposits_and_ignored_logs() {
    let fixture = Fixture::load();
    let limbo_epoch = fixture
        .epochs
        .iter()
        .position(|epoch| epoch.limbo_sat > 0)
        .expect("fixture limbos deposits");
    let ignored_types = [2u16, 11, 12, 13];
    let logged_types: Vec<_> = fixture.epochs[limbo_epoch]
        .manifests()
        .flat_map(|manifest| {
            manifest
                .logs()
                .iter()
                .filter_map(|log| log.ty())
                .collect::<Vec<_>>()
        })
        .collect();
    for ty in ignored_types {
        assert!(
            logged_types.contains(&ty),
            "fixture carries a type {ty} log"
        );
    }

    let mut node = Node::new(&fixture.params, &fixture.epochs);
    let before = node.replay_v0(&fixture.epochs[..limbo_epoch]);
    let after = node.replay_v0(&fixture.epochs[limbo_epoch..=limbo_epoch]);

    let limbo = |artifacts: &AppliedEpochArtifacts| {
        MemoryStateBaseLayer::from_container(artifacts.new_state.clone())
            .limbo_funds()
            .to_sat()
    };
    assert_eq!(limbo(&before), 0);
    assert_eq!(limbo(&after), 400_000_000);
}

/// The EE key update took effect in the drain, as under V0, instead of
/// reaching the EE account's inbox as V1 rules deliver it.
#[test]
fn test_v0_ee_key_update_applies_at_once() {
    let fixture = Fixture::load();
    let mut node = Node::new(&fixture.params, &fixture.epochs);
    let key_update = fixture
        .epochs
        .iter()
        .position(|epoch| epoch.epoch == fixture.ee_key_update_epoch)
        .expect("fixture has the key update epoch");
    let before = node.replay_v0(&fixture.epochs[..key_update]);
    let after = node.replay_v0(&fixture.epochs[key_update..=key_update]);

    let ee_account = |artifacts: &AppliedEpochArtifacts| {
        MemoryStateBaseLayer::from_container(artifacts.new_state.clone())
            .get_account_state(fixture.ee_account())
            .expect("read EE account")
            .expect("EE account exists")
            .clone()
    };
    let before = ee_account(&before);
    let after = ee_account(&after);
    let before = before.as_snark_account().expect("EE is a snark account");
    let after = after.as_snark_account().expect("EE is a snark account");

    assert_eq!(after.update_vk(), &fixture.rotated_ee_key());
    assert_ne!(before.update_vk(), after.update_vk());
    assert_eq!(
        after.inbox_mmr().num_entries(),
        before.inbox_mmr().num_entries(),
        "the key update adds no inbox message"
    );
}

/// The epoch after the enactment runs V1 and wraps the state, then a V1 epoch
/// with no manifests follows on the V1 state, each matching block execution.
#[test]
fn test_epochs_after_enactment_run_v1() {
    let fixture = Fixture::load();
    let mut node = Node::new(&fixture.params, &fixture.epochs);
    let last_v0 = node.replay_v0(&fixture.epochs);

    let first_v1 = build_first_v1_epoch(&fixture, &last_v0);
    node.add_epoch(&first_v1);
    let applied = node
        .apply(first_v1.epoch_commitment)
        .expect("first V1 epoch applies");
    assert_eq!(
        applied.new_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V1)
    );
    assert_eq!(
        *applied.summary.final_state(),
        *first_v1.terminal_header.state_root()
    );
    assert_eq!(applied.terminal_header, first_v1.terminal_header);
    assert_eq!(applied.new_state, first_v1.post_state);
    assert_ne!(
        *applied.summary.final_state(),
        applied.new_state.chainstate().compute_chainstate_root(),
        "V1 roots wrap the chainstate root"
    );

    let second_v1 = build_v1_epoch(
        &fixture.params,
        &applied.new_state,
        &applied.terminal_header,
        Vec::new(),
    );
    node.add_epoch(&second_v1);
    let applied = node
        .apply(second_v1.epoch_commitment)
        .expect("V1 epoch on a V1 state applies");
    assert_eq!(
        applied.new_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V1)
    );
    assert_eq!(applied.terminal_header, second_v1.terminal_header);
    assert_eq!(applied.new_state, second_v1.post_state);
}

/// A first V1 epoch with no manifests still runs V1 and wraps, and the next
/// empty epoch, whose state keeps the enactment height as its last L1 height,
/// stays V1.
#[test]
fn test_empty_epochs_after_enactment_run_v1() {
    let fixture = Fixture::load();
    let mut node = Node::new(&fixture.params, &fixture.epochs);
    let mut parent = node.replay_v0(&fixture.epochs);
    let enactment_height = fixture.last_v0_epoch().l1_height;

    for _ in 0..2 {
        let built = build_v1_epoch(
            &fixture.params,
            &parent.new_state,
            &parent.terminal_header,
            Vec::new(),
        );
        node.add_epoch(&built);
        let mut restarted = node.restarted_at(parent.terminal_header.epoch());
        let applied = restarted
            .apply(built.epoch_commitment)
            .expect("empty V1 epoch applies after restart");
        assert_eq!(
            applied.new_state.spec_versions(),
            OLSpecVersions::uniform(OLSpecId::V1)
        );
        assert_eq!(applied.terminal_header, built.terminal_header);
        assert_eq!(applied.new_state, built.post_state);
        assert_eq!(
            applied.new_state.chainstate().last_l1_block().height(),
            enactment_height
        );
        parent = node
            .apply(built.epoch_commitment)
            .expect("empty V1 epoch applies");
    }
}

/// A node restarted at the last V0 epoch selects V1 for the next epoch from
/// what it stored, as a node that kept running does.
#[test]
fn test_spec_selection_survives_restart_at_enactment() {
    let fixture = Fixture::load();
    let mut node = Node::new(&fixture.params, &fixture.epochs);
    let last_v0 = node.replay_v0(&fixture.epochs);
    let first_v1 = build_first_v1_epoch(&fixture, &last_v0);
    node.add_epoch(&first_v1);

    let mut restarted = node.restarted_at(fixture.last_v0_epoch().epoch);
    let after_restart = restarted
        .apply(first_v1.epoch_commitment)
        .expect("first V1 epoch applies after restart");
    let without_restart = node
        .apply(first_v1.epoch_commitment)
        .expect("first V1 epoch applies");

    assert_eq!(after_restart.new_state, without_restart.new_state);
    assert_eq!(
        after_restart.terminal_header,
        without_restart.terminal_header
    );
}

/// Replaces the stored manifest at the enactment height and returns the error
/// the first V1 epoch then fails with.
fn apply_first_v1_epoch_with_enactment_manifest(
    replacement: impl FnOnce(&AsmManifest) -> Option<AsmManifest>,
) -> WorkerError {
    let fixture = Fixture::load();
    let mut node = Node::new(&fixture.params, &fixture.epochs);
    let last_v0 = node.replay_v0(&fixture.epochs);
    let first_v1 = build_first_v1_epoch(&fixture, &last_v0);
    node.add_epoch(&first_v1);

    let height = fixture.last_v0_epoch().l1_height;
    let enactment = node
        .ctx
        .manifests
        .remove(&height)
        .expect("enactment manifest");
    if let Some(replacement) = replacement(&enactment) {
        node.ctx.manifests.insert(height, replacement);
    }
    match node.apply(first_v1.epoch_commitment) {
        Ok(_) => panic!("first V1 epoch must not apply"),
        Err(err) => err,
    }
}

#[test]
fn test_reorged_last_manifest_is_rejected() {
    let err = apply_first_v1_epoch_with_enactment_manifest(|enactment| {
        Some(
            AsmManifest::new(
                enactment.height(),
                L1BlockId::from(Buf32::from([0xee; 32])),
                *enactment.wtxids_root(),
                enactment.logs().to_vec(),
            )
            .expect("manifest"),
        )
    });
    assert!(
        matches!(err, WorkerError::LastManifestMismatch { .. }),
        "{err}"
    );
}

#[test]
fn test_missing_last_manifest_is_an_error() {
    let fixture = Fixture::load();
    let enactment_height = fixture.last_v0_epoch().l1_height;
    let err = apply_first_v1_epoch_with_enactment_manifest(|_| None);
    assert!(
        matches!(err, WorkerError::MissingLastManifest { height } if height == enactment_height),
        "{err}"
    );
}

/// An enactment log in the last manifest of an earlier V0 epoch selects V1
/// for the next epoch, one switch too early. The replay then fails on the
/// root form instead of accepting a diverging state; this is why V0 history
/// must hold no enactment except the switch.
#[test]
fn test_enactment_before_the_switch_fails_the_replay() {
    let fixture = Fixture::load();
    let mut node = Node::new(&fixture.params, &fixture.epochs);
    // The parent epoch must have processed a manifest, so that its last
    // manifest is the one the selection reads.
    let parent = fixture
        .epochs
        .iter()
        .position(|epoch| !epoch.manifests.is_empty())
        .expect("an epoch processes a manifest");
    node.replay_v0(&fixture.epochs[..=parent]);

    let height = fixture.epochs[parent].l1_height;
    let last_manifest = node.ctx.manifests[&height].clone();
    let mut logs = last_manifest.logs().to_vec();
    logs.extend(
        make_checkpoint_predicate_enactment_manifest(height, 1)
            .logs()
            .iter()
            .cloned(),
    );
    let with_enactment = AsmManifest::new(
        height,
        *last_manifest.blkid(),
        *last_manifest.wtxids_root(),
        logs,
    )
    .expect("manifest");
    node.ctx.manifests.insert(height, with_enactment);

    let next = &fixture.epochs[parent + 1];
    let err = match node.apply(next.epoch_commitment()) {
        Ok(_) => panic!("the next epoch must not apply under V1"),
        Err(err) => err,
    };
    assert!(
        matches!(err, WorkerError::TerminalBlkidMismatch { .. }),
        "{err}"
    );
}

/// Without the enactment, the node replays the epoch under V0. The root form
/// then differs from the checkpoint's, so the replay fails instead of
/// accepting a diverging state.
#[test]
fn test_wrong_spec_selection_fails_instead_of_diverging() {
    let err = apply_first_v1_epoch_with_enactment_manifest(|enactment| {
        Some(
            AsmManifest::new(
                enactment.height(),
                *enactment.blkid(),
                *enactment.wtxids_root(),
                Vec::new(),
            )
            .expect("manifest"),
        )
    });
    assert!(
        matches!(err, WorkerError::TerminalBlkidMismatch { .. }),
        "{err}"
    );
}
