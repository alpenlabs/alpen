//! Differential test across the STF drivers that share blocks: block
//! construction, block verification, and checkpoint DA replay.
//!
//! Blocks built through [`construct_block`] must verify through
//! [`verify_block`] with the same logs and state roots, and each epoch's DA,
//! computed by [`compute_epoch_da`], must reproduce the epoch's terminal state
//! root through both [`apply_da_epoch`] and [`verify_epoch_with_diff`].

use std::iter;

use strata_acct_types::BitcoinAmount;
use strata_identifiers::{L1Height, SubjectId};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::IStateAccessor;
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, EpochDaReplayError, EpochExecExpectations, EpochInfo,
    ExecError, OLSpecId, apply_da_epoch, construct_block, verify_block, verify_epoch_with_diff,
};
use strata_ol_stf_v1::test_utils::*;
use strata_ol_tx_types_v1::{OLTransactionDataV1, OLTransactionV1, TxProofsV1};

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
}

/// Builds a chain through [`construct_block`], tracking epoch boundaries.
struct ChainBuilder {
    state: MemoryStateBaseLayer<OLStateV1>,
    runtime_params: OLRuntimeParams,
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
            SPEC,
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
