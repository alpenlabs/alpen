//! Differential test across the STF drivers that share blocks: block
//! construction, block verification, checkpoint DA replay, and the checkpoint
//! proof program.
//!
//! Blocks built through [`construct_block`] must verify through
//! [`verify_block`] with the same logs and state roots, and each epoch's DA,
//! computed by [`compute_epoch_da`], must reproduce the epoch's terminal state
//! root through both [`apply_da_epoch`] and [`verify_epoch_with_diff`]. The
//! checkpoint program must prove each epoch from its start state container.
//! The drivers must also agree on the first V1 epoch after a V0 state, which
//! wraps it, and on the first V2 epoch after a V1 enactment, which takes over
//! the V1 state. No checkpoint program proves V2 epochs yet, and the V1
//! program refuses the first one.

use std::iter;

use strata_acct_types::BitcoinAmount;
use strata_identifiers::{Buf64, L1Height, SubjectId};
use strata_ol_chain_types_v1::{
    AsmManifest, OLBlockHeaderV1, OLBlockV1, OLLog, SignedOLBlockHeaderV1,
};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{MemoryStateBaseLayer, WriteTrackingState};
use strata_ol_state_types::{IStateAccessor, IStateAccessorMut, OLSpecVersions};
use strata_ol_state_types_v1::{IStateBatchApplicable, OLStateV1};
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, EpochDaReplayError, EpochExecExpectations, EpochInfo,
    ExecError, OLSpecId, apply_da_epoch, construct_block, verify_block, verify_epoch_with_diff,
};
use strata_ol_stf_v1::test_utils::*;
use strata_ol_tx_types_v1::{OLTransactionDataV1, OLTransactionV1, TxProofsV1};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};

use crate::compute_epoch_da;

const SPEC: OLSpecId = OLSpecId::V1;

/// A block built by [`construct_block`], with the logs construction emitted.
struct BuiltBlock {
    block: OLBlockV1,
    logs: Vec<OLLog>,
}

/// One epoch of built blocks and the state it started from.
struct BuiltEpoch {
    pre_epoch_state: MemoryStateBaseLayer<OLStateV1>,
    previous_terminal: OLBlockHeaderV1,
    blocks: Vec<BuiltBlock>,
}

impl BuiltEpoch {
    fn ol_blocks(&self) -> Vec<OLBlockV1> {
        self.blocks
            .iter()
            .map(|built| built.block.clone())
            .collect()
    }

    fn terminal(&self) -> &OLBlockHeaderV1 {
        self.blocks
            .last()
            .expect("epoch has a terminal block")
            .block
            .header()
    }

    fn manifests(&self) -> Vec<AsmManifest> {
        self.blocks
            .iter()
            .filter_map(|built| built.block.body().manifests())
            .flat_map(|container| container.manifests().iter().cloned())
            .collect()
    }

    fn logs(&self) -> Vec<OLLog> {
        self.blocks
            .iter()
            .flat_map(|built| built.logs.iter().cloned())
            .collect()
    }

    /// Returns the checkpoint proof input for this epoch with its encoded DA.
    fn prover_input(&self, da_state_diff_bytes: Vec<u8>) -> CheckpointProverInput {
        CheckpointProverInput {
            start_state: self.pre_epoch_state.to_container(),
            blocks: self.ol_blocks(),
            parent: self.previous_terminal.clone(),
            da_state_diff_bytes,
        }
    }
}

/// Builds a chain through [`construct_block`], tracking epoch boundaries.
struct ChainBuilder {
    state: MemoryStateBaseLayer<OLStateV1>,
    runtime_params: OLRuntimeParams,
    /// The spec the next block is built under. Genesis is built under
    /// [`SPEC`].
    spec: OLSpecId,
    genesis: BuiltBlock,
    epochs: Vec<BuiltEpoch>,
    current: Option<BuiltEpoch>,
}

impl ChainBuilder {
    /// Executes genesis on top of `pre_genesis_state`.
    fn new(pre_genesis_state: MemoryStateBaseLayer<OLStateV1>) -> Self {
        let mut state = pre_genesis_state;
        let runtime_params = OLRuntimeParams::test_default();
        let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);
        let output = construct_block(
            SPEC,
            &mut state,
            BlockContext::new(&genesis_info, None),
            BlockComponents::new_manifests(vec![make_empty_manifest(1, 0)]).as_terminal(),
            &runtime_params,
        )
        .expect("genesis constructs");
        let genesis = BuiltBlock {
            block: to_ol_block(output.completed_block()),
            logs: output.outputs().logs().to_vec(),
        };
        Self {
            state,
            runtime_params,
            spec: SPEC,
            genesis,
            epochs: Vec::new(),
            current: None,
        }
    }

    fn parent_header(&self) -> &OLBlockHeaderV1 {
        self.current
            .as_ref()
            .and_then(|epoch| epoch.blocks.last())
            .or_else(|| self.epochs.last().and_then(|epoch| epoch.blocks.last()))
            .unwrap_or(&self.genesis)
            .block
            .header()
    }

    /// Constructs the next block from `components`.
    fn push(&mut self, components: BlockComponents) {
        let parent = self.parent_header().clone();
        let epoch = self.current.get_or_insert_with(|| BuiltEpoch {
            pre_epoch_state: self.state.clone(),
            previous_terminal: parent.clone(),
            blocks: Vec::new(),
        });

        let slot = parent.slot() + 1;
        let block_epoch = parent.epoch() + u32::from(parent.is_terminal());
        let block_info = BlockInfo::new(
            EPOCH_RUNNER_GENESIS_TIMESTAMP + slot * EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
            slot,
            block_epoch,
        );
        let is_terminal = components.is_terminal();
        let output = construct_block(
            self.spec,
            &mut self.state,
            BlockContext::new(&block_info, Some(&parent)),
            components,
            &self.runtime_params,
        )
        .expect("block constructs");
        epoch.blocks.push(BuiltBlock {
            block: to_ol_block(output.completed_block()),
            logs: output.outputs().logs().to_vec(),
        });

        if is_terminal {
            self.epochs
                .push(self.current.take().expect("current epoch was just set"));
        }
    }
}

fn gam_components(target_index: u32, payload: Vec<u8>) -> BlockComponents {
    let tx = OLTransactionV1::new(
        OLTransactionDataV1::from_gam_bytes(make_account_id(target_index), payload)
            .expect("GAM payload fits"),
        TxProofsV1::new_empty(),
    );
    BlockComponents::new_txs_from_ol_transactions(vec![tx])
}

fn manifest_components(manifest: AsmManifest, is_terminal: bool) -> BlockComponents {
    BlockComponents::new_manifests(vec![manifest]).with_terminal(is_terminal)
}

/// Builds three epochs covering inbox delivery, a snark account update, a
/// deposit, a manifest in a non-terminal block, and a checkpoint predicate
/// boundary at an epoch terminal.
fn build_chain() -> (MemoryStateBaseLayer<OLStateV1>, ChainBuilder) {
    let mut pre_genesis_state = make_genesis_state();
    let snark_serial = epoch_runner_seed_accounts(&mut pre_genesis_state);
    let mut chain = ChainBuilder::new(pre_genesis_state.clone());
    let mut next_l1_height: L1Height = 2;
    let mut next_manifest_height = || {
        let height = next_l1_height;
        next_l1_height += 1;
        height
    };

    // Epoch 1: deliver an inbox message, consume it with a snark account
    // update, buffer a manifest early, and end with a deposit.
    let inbox_msg = snark_inbox_msg();
    chain.push(gam_components(
        TEST_SNARK_ACCOUNT_ID,
        inbox_msg.payload().data().to_vec(),
    ));
    let update_tx = build_snark_update(&chain.state, &inbox_msg);
    chain.push(BlockComponents::new_txs_from_ol_transactions(vec![
        update_tx,
    ]));
    chain.push(manifest_components(
        make_empty_manifest(next_manifest_height(), 1),
        false,
    ));
    chain.push(manifest_components(
        make_deposit_manifest_for_account(
            next_manifest_height(),
            2,
            snark_serial,
            SubjectId::from([7u8; 32]),
            BitcoinAmount::try_from(150_000_000)
                .expect("amount must not exceed the Bitcoin money supply"),
        ),
        true,
    ));

    // Epoch 2: ends at a checkpoint predicate boundary.
    chain.push(gam_components(TEST_RECIPIENT_ID, b"to recipient".to_vec()));
    chain.push(manifest_components(
        make_checkpoint_predicate_enactment_manifest(next_manifest_height(), 1),
        true,
    ));

    // Epoch 3: the first epoch after the boundary.
    chain.push(BlockComponents::new_empty());
    chain.push(manifest_components(
        make_empty_manifest(next_manifest_height(), 3),
        true,
    ));

    assert!(
        chain.current.is_none(),
        "chain must end at an epoch terminal"
    );
    (pre_genesis_state, chain)
}

#[test]
fn test_constructed_blocks_verify_with_same_logs_and_roots() {
    let (pre_genesis_state, chain) = build_chain();
    let mut state = pre_genesis_state;

    let mut parent = None;
    let blocks =
        iter::once(&chain.genesis).chain(chain.epochs.iter().flat_map(|epoch| epoch.blocks.iter()));
    for built in blocks {
        let header = built.block.header();
        let logs = verify_block(
            SPEC,
            &mut state,
            header,
            parent,
            built.block.body(),
            &chain.runtime_params,
        )
        .expect("constructed block verifies");

        assert_eq!(logs, built.logs, "logs differ at slot {}", header.slot());
        assert_eq!(
            state.compute_state_root().expect("state root"),
            *header.state_root(),
            "state root differs at slot {}",
            header.slot()
        );
        parent = Some(header);
    }
}

#[test]
fn test_epoch_da_replay_reproduces_terminal_roots() {
    let (_, chain) = build_chain();
    assert_eq!(chain.epochs.len(), 3);

    for epoch in &chain.epochs {
        let terminal = epoch.terminal();
        let (encoded_da, da_logs) = compute_epoch_da(
            SPEC,
            epoch.pre_epoch_state.clone(),
            &epoch.ol_blocks(),
            &epoch.previous_terminal,
            &chain.runtime_params,
        )
        .expect("epoch DA computes")
        .into_parts();

        // DA replay skips the terminal drain. The drain emits no OL logs, so the
        // pre-drain logs equal the logs block construction emitted.
        assert_eq!(da_logs, epoch.logs(), "epoch {} logs", terminal.epoch());

        let epoch_info = EpochInfo::new(
            BlockInfo::from_header(terminal),
            epoch.previous_terminal.compute_block_commitment(),
        );
        let manifests = epoch.manifests();

        let mut applied_state = epoch.pre_epoch_state.clone();
        apply_da_epoch(
            SPEC,
            &mut applied_state,
            &epoch_info,
            &encoded_da,
            &manifests,
            &chain.runtime_params,
        )
        .expect("epoch DA applies");
        assert_eq!(
            applied_state.compute_state_root().expect("state root"),
            *terminal.state_root(),
            "epoch {} DA replay root",
            terminal.epoch()
        );

        let mut verified_state = epoch.pre_epoch_state.clone();
        verify_epoch_with_diff(
            SPEC,
            &mut verified_state,
            &epoch_info,
            &encoded_da,
            &manifests,
            &EpochExecExpectations::new(*terminal.state_root()),
            &chain.runtime_params,
        )
        .expect("epoch DA verifies against the terminal root");

        let mut mismatched_state = epoch.pre_epoch_state.clone();
        let err = verify_epoch_with_diff(
            SPEC,
            &mut mismatched_state,
            &epoch_info,
            &encoded_da,
            &manifests,
            &EpochExecExpectations::new(*epoch.previous_terminal.state_root()),
            &chain.runtime_params,
        )
        .expect_err("epoch DA must not verify against another root");
        assert!(matches!(
            err,
            EpochDaReplayError::Exec(ExecError::ChainIntegrity)
        ));
    }
}

#[test]
fn test_checkpoint_program_proves_each_epoch() {
    let (_, chain) = build_chain();

    for epoch in &chain.epochs {
        let terminal = epoch.terminal();
        let blocks = epoch.ol_blocks();
        let (da_state_diff_bytes, _) = compute_epoch_da(
            SPEC,
            epoch.pre_epoch_state.clone(),
            &blocks,
            &epoch.previous_terminal,
            &chain.runtime_params,
        )
        .expect("epoch DA computes")
        .into_parts();

        let claim = CheckpointProgram::execute(
            &epoch.prover_input(da_state_diff_bytes),
            SPEC,
            chain.runtime_params,
        )
        .expect("checkpoint program proves the epoch");
        assert_eq!(claim.epoch(), terminal.epoch());
        assert_eq!(
            *claim.l2_range().end().blkid(),
            terminal.compute_blkid(),
            "epoch {} claim must end at its terminal",
            terminal.epoch()
        );
    }
}

/// Builds genesis under V1, relabels its result as the last V0 terminal, and
/// builds one V1 epoch on it.
///
/// The relabelled terminal keeps genesis's chainstate as a V0 state, under a
/// header committing to its bare root, as MN0's last 0.3.0 terminal does.
fn build_chain_from_v0_terminal() -> ChainBuilder {
    let mut pre_genesis_state = make_genesis_state();
    epoch_runner_seed_accounts(&mut pre_genesis_state);
    let mut chain = ChainBuilder::new(pre_genesis_state);

    chain
        .state
        .set_spec_versions(OLSpecVersions::uniform(OLSpecId::V0));
    let v0_root = chain.state.compute_state_root().expect("V0 root");
    let header = tamper_state_root(chain.genesis.block.header(), v0_root);
    let body = chain.genesis.block.body().clone();
    chain.genesis.block = OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body);

    chain.push(gam_components(
        TEST_RECIPIENT_ID,
        b"first V1 epoch".to_vec(),
    ));
    chain.push(manifest_components(make_empty_manifest(2, 1), true));
    chain
}

/// Checks that block verification and checkpoint DA replay agree with block
/// construction on `epoch`, the chain's last epoch and the first under
/// `spec`, which takes over the state of the spec before it.
///
/// Returns the epoch's encoded DA.
fn assert_drivers_agree_on_takeover_epoch(
    chain: &ChainBuilder,
    epoch: &BuiltEpoch,
    spec: OLSpecId,
) -> Vec<u8> {
    let terminal = epoch.terminal();
    let taken_over = OLSpecVersions::uniform(spec);

    // Block construction took over the state.
    assert_eq!(chain.state.spec_versions(), taken_over);
    assert_eq!(
        chain.state.compute_state_root().expect("terminal root"),
        *terminal.state_root()
    );

    // Node execution verifies each block on a write-tracking overlay and
    // applies its batch, as the chain worker does, so the takeover travels in
    // the first block's batch.
    let mut node_state = epoch.pre_epoch_state.clone();
    let mut parent = &epoch.previous_terminal;
    for built in &epoch.blocks {
        let header = built.block.header();
        let mut tracking = WriteTrackingState::new_empty(&node_state);
        let logs = verify_block(
            spec,
            &mut tracking,
            header,
            Some(parent),
            built.block.body(),
            &chain.runtime_params,
        )
        .expect("block verifies");
        assert_eq!(logs, built.logs, "logs differ at slot {}", header.slot());
        let batch = tracking.into_batch();
        node_state.apply_write_batch(batch).expect("batch applies");
        assert_eq!(
            node_state.compute_state_root().expect("node root"),
            *header.state_root(),
            "node root differs at slot {}",
            header.slot()
        );
        parent = header;
    }
    assert_eq!(node_state.spec_versions(), taken_over);

    // DA replay from the epoch's start state takes over the state the same
    // way.
    let (encoded_da, _) = compute_epoch_da(
        spec,
        epoch.pre_epoch_state.clone(),
        &epoch.ol_blocks(),
        &epoch.previous_terminal,
        &chain.runtime_params,
    )
    .expect("epoch DA computes")
    .into_parts();
    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal),
        epoch.previous_terminal.compute_block_commitment(),
    );
    let mut replayed = epoch.pre_epoch_state.clone();
    apply_da_epoch(
        spec,
        &mut replayed,
        &epoch_info,
        &encoded_da,
        &epoch.manifests(),
        &chain.runtime_params,
    )
    .expect("epoch DA applies");
    assert_eq!(replayed.spec_versions(), taken_over);
    assert_eq!(
        replayed.compute_state_root().expect("replayed root"),
        *terminal.state_root()
    );
    verify_epoch_with_diff(
        spec,
        &mut epoch.pre_epoch_state.clone(),
        &epoch_info,
        &encoded_da,
        &epoch.manifests(),
        &EpochExecExpectations::new(*terminal.state_root()),
        &chain.runtime_params,
    )
    .expect("epoch DA verifies against the terminal root");
    encoded_da
}

#[test]
fn test_drivers_agree_on_first_v1_epoch_after_v0() {
    let chain = build_chain_from_v0_terminal();
    let [epoch] = chain.epochs.as_slice() else {
        panic!("chain has one epoch after the V0 terminal");
    };

    // The epoch starts from the V0 terminal, authenticated by its bare root.
    assert_eq!(
        epoch.pre_epoch_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V0)
    );
    assert_eq!(
        epoch.pre_epoch_state.compute_state_root().expect("V0 root"),
        epoch.pre_epoch_state.chainstate().compute_chainstate_root()
    );
    assert_eq!(
        epoch.pre_epoch_state.compute_state_root().expect("V0 root"),
        *epoch.previous_terminal.state_root()
    );

    let encoded_da = assert_drivers_agree_on_takeover_epoch(&chain, epoch, SPEC);

    // The checkpoint program proves the epoch from the V0 start container.
    let claim =
        CheckpointProgram::execute(&epoch.prover_input(encoded_da), SPEC, chain.runtime_params)
            .expect("checkpoint program proves the first V1 epoch");
    assert_eq!(
        *claim.l2_range().end().blkid(),
        epoch.terminal().compute_blkid()
    );
}

/// Builds genesis and a V1 epoch that processes a checkpoint predicate
/// enactment at its terminal, then the first V2 epoch, which runs V2 as the
/// node selects it after that enactment.
fn build_chain_into_v2() -> ChainBuilder {
    let mut pre_genesis_state = make_genesis_state();
    epoch_runner_seed_accounts(&mut pre_genesis_state);
    let mut chain = ChainBuilder::new(pre_genesis_state);

    chain.push(gam_components(TEST_RECIPIENT_ID, b"last V1 epoch".to_vec()));
    chain.push(manifest_components(
        make_checkpoint_predicate_enactment_manifest(2, 1),
        true,
    ));

    chain.spec = OLSpecId::V2;
    chain.push(gam_components(
        TEST_RECIPIENT_ID,
        b"first V2 epoch".to_vec(),
    ));
    chain.push(BlockComponents::new_empty());
    chain.push(manifest_components(make_empty_manifest(3, 1), true));
    chain
}

#[test]
fn test_drivers_agree_on_first_v2_epoch_after_v1() {
    let chain = build_chain_into_v2();
    let [_, epoch] = chain.epochs.as_slice() else {
        panic!("chain has the last V1 epoch and the first V2 epoch");
    };

    // The epoch starts from the V1 terminal that processed the enactment,
    // which staged no successor.
    assert_eq!(
        epoch.pre_epoch_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V1)
    );
    assert_eq!(
        epoch.pre_epoch_state.compute_state_root().expect("V1 root"),
        *epoch.previous_terminal.state_root()
    );

    assert_drivers_agree_on_takeover_epoch(&chain, epoch, OLSpecId::V2);
    // TODO(STR-4662): prove the epoch with the V2 checkpoint program from its
    // V1 start container, as the first V1 epoch is proven from its V0 one.

    // The first V2 block commits V2 in its root, so V1 rules do not
    // reproduce it.
    let first = &epoch.blocks[0].block;
    let err = verify_block(
        OLSpecId::V1,
        &mut epoch.pre_epoch_state.clone(),
        first.header(),
        Some(&epoch.previous_terminal),
        first.body(),
        &chain.runtime_params,
    )
    .expect_err("V1 rules must not verify the first V2 block");
    assert!(matches!(err, ExecError::ChainIntegrity), "{err}");
}

#[test]
#[should_panic(expected = "computed block header does not match input block header")]
fn test_v1_program_refuses_the_first_v2_epoch() {
    let chain = build_chain_into_v2();
    let [_, epoch] = chain.epochs.as_slice() else {
        panic!("chain has the last V1 epoch and the first V2 epoch");
    };
    let (da_state_diff_bytes, _) = compute_epoch_da(
        OLSpecId::V2,
        epoch.pre_epoch_state.clone(),
        &epoch.ol_blocks(),
        &epoch.previous_terminal,
        &chain.runtime_params,
    )
    .expect("epoch DA computes")
    .into_parts();

    // The V1 terminal stages V1, so the program runs the epoch under V1, and
    // the first block's root, which commits V2, does not match.
    let _ = CheckpointProgram::execute(
        &epoch.prover_input(da_state_diff_bytes),
        SPEC,
        chain.runtime_params,
    );
}
