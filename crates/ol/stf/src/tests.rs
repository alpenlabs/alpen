//! Smoke tests for dispatch under [`OLSpecId::V1`].
//!
//! The differential tests in `strata-ol-checkpoint` and
//! `strata-ol-block-assembly` compare the drivers against each other.

use std::iter;

use strata_ol_chain_types_v1::OLBlockV1;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::IStateAccessor;
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf_v1::test_utils::*;
use strata_ol_tx_types_v1::{OLTransactionDataV1, OLTransactionV1, TxProofsV1};

use crate::{
    BlockComponents, BlockInfo, EpochDaReplayError, EpochInfo, OLSpecId, apply_da_epoch,
    verify_block,
};

/// An epoch built with the V1 STF, with its pre-genesis and pre-epoch states.
struct V1Epoch {
    pre_genesis_state: MemoryStateBaseLayer<OLStateV1>,
    pre_epoch_state: MemoryStateBaseLayer<OLStateV1>,
    genesis: OLBlockV1,
    epoch_blocks: Vec<OLBlockV1>,
}

/// Builds genesis and one epoch that delivers an inbox message, consumes it
/// with a snark account update, and ends with a terminal manifest.
fn build_v1_epoch() -> V1Epoch {
    let mut state = make_genesis_state();
    epoch_runner_seed_accounts(&mut state);
    let pre_genesis_state = state.clone();
    let genesis = epoch_runner_run_genesis(&mut state);
    let pre_epoch_state = state.clone();

    let inbox_msg = snark_inbox_msg();
    let gam_tx = OLTransactionV1::new(
        OLTransactionDataV1::from_gam_bytes(
            make_account_id(TEST_SNARK_ACCOUNT_ID),
            inbox_msg.payload().data().to_vec(),
        )
        .expect("GAM payload fits"),
        TxProofsV1::new_empty(),
    );

    let mut epoch_blocks = Vec::new();
    let mut parent = epoch_runner_run_block(
        &mut state,
        &mut epoch_blocks,
        genesis.header(),
        BlockComponents::new_txs_from_ol_transactions(vec![gam_tx]),
    );
    let update_tx = build_snark_update(&state, &inbox_msg);
    parent = epoch_runner_run_block(
        &mut state,
        &mut epoch_blocks,
        &parent,
        BlockComponents::new_txs_from_ol_transactions(vec![update_tx]),
    );
    epoch_runner_run_terminal(
        &mut state,
        &mut epoch_blocks,
        &parent,
        make_empty_manifest(EPOCH_RUNNER_TERMINAL_L1_HEIGHT, 0),
    );

    V1Epoch {
        pre_genesis_state,
        pre_epoch_state,
        genesis: to_ol_block(&genesis),
        epoch_blocks,
    }
}

#[test]
fn test_verify_block_matches_v1() {
    let epoch = build_v1_epoch();
    let runtime_params = OLRuntimeParams::test_default();
    let mut dispatched_state = epoch.pre_genesis_state.clone();
    let mut direct_state = epoch.pre_genesis_state;

    let mut parent = None;
    for block in iter::once(&epoch.genesis).chain(&epoch.epoch_blocks) {
        let dispatched_logs = verify_block(
            OLSpecId::V1,
            &mut dispatched_state,
            block.header(),
            parent,
            block.body(),
            &runtime_params,
        )
        .expect("dispatched verification succeeds");
        let direct_logs = strata_ol_stf_v1::verify_block(
            &mut direct_state,
            block.header(),
            parent,
            block.body(),
            &runtime_params,
        )
        .expect("direct verification succeeds");

        assert_eq!(dispatched_logs, direct_logs);
        assert_eq!(
            dispatched_state.compute_state_root().expect("state root"),
            direct_state.compute_state_root().expect("state root"),
        );
        parent = Some(block.header());
    }
}

#[test]
fn test_apply_da_epoch_distinguishes_decode_failure() {
    let epoch = build_v1_epoch();
    let terminal = epoch.epoch_blocks.last().expect("epoch has a terminal");
    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal.header()),
        epoch.genesis.header().compute_block_commitment(),
    );
    let mut state = epoch.pre_epoch_state;

    let err = apply_da_epoch(
        OLSpecId::V1,
        &mut state,
        &epoch_info,
        &[0xff; 3],
        &[],
        &OLRuntimeParams::test_default(),
    )
    .expect_err("malformed DA bytes must not decode");

    assert!(matches!(err, EpochDaReplayError::Decode(_)));
}
