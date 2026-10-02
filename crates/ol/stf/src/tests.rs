//! Smoke tests for dispatch under [`OLSpecId::V1`], and for [`OLSpecId::V0`],
//! whose rules this binary implements only for genesis.
//!
//! The differential tests in `strata-ol-checkpoint` and
//! `strata-ol-block-assembly` compare the drivers against each other.

use std::iter;

use strata_acct_types::BitcoinAmount;
use strata_ol_chain_types_v1::OLBlockV1;
use strata_ol_params::{OLParams, OLRuntimeParams};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IStateAccessor, OLSpecVersions};
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf_v1::process_block_manifests;
use strata_ol_stf_v1::test_utils::*;
use strata_ol_tx_types_v1::{
    OLTransactionDataV1, OLTransactionV1, TransactionPayloadV1, TxProofsV1,
};

use crate::{
    BasicExecContext, BlockComponents, BlockContext, BlockInfo, CompletedBlock, EpochDaReplayError,
    EpochExecExpectations, EpochInfo, ExecError, ExecOutputBuffer, ExecResult, OLSpecId,
    TxExecContext, apply_da_epoch, construct_block, execute_and_complete_block,
    execute_block_batch_predrain, sequencer, verify_block, verify_epoch_with_diff,
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

#[test]
fn test_verify_block_under_v0_is_unimplemented() {
    let epoch = build_v1_epoch();
    let mut state = epoch.pre_genesis_state.clone();

    let err = verify_block(
        OLSpecId::V0,
        &mut state,
        epoch.genesis.header(),
        None,
        epoch.genesis.body(),
        &OLRuntimeParams::test_default(),
    )
    .expect_err("V0 block verification is not implemented");

    assert!(matches!(err, ExecError::UnimplementedSpec(OLSpecId::V0)));
    assert_eq!(
        state.compute_state_root().expect("state root"),
        epoch
            .pre_genesis_state
            .compute_state_root()
            .expect("state root"),
        "a rejected spec must not touch the state"
    );
}

#[test]
fn test_apply_da_epoch_under_v0_is_unimplemented() {
    let epoch = build_v1_epoch();
    let terminal = epoch.epoch_blocks.last().expect("epoch has a terminal");
    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal.header()),
        epoch.genesis.header().compute_block_commitment(),
    );
    let mut state = epoch.pre_epoch_state;

    // The spec is rejected before its DA encoding is chosen, so even bytes
    // that would not decode report the unimplemented spec.
    let err = apply_da_epoch(
        OLSpecId::V0,
        &mut state,
        &epoch_info,
        &[0xff; 3],
        &[],
        &OLRuntimeParams::test_default(),
    )
    .expect_err("V0 DA replay is not implemented");

    assert!(matches!(
        err,
        EpochDaReplayError::Exec(ExecError::UnimplementedSpec(OLSpecId::V0))
    ));
}

/// Executes the genesis block 0.3.0 networks run, with no transactions or
/// manifests, on the genesis state of `params` under `spec`.
fn run_empty_genesis(
    spec: OLSpecId,
    params: &OLParams,
) -> (MemoryStateBaseLayer<OLStateV1>, ExecResult<CompletedBlock>) {
    let mut state = MemoryStateBaseLayer::new_genesis(params).expect("genesis state");
    let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);
    let result = execute_and_complete_block(
        spec,
        &mut state,
        BlockContext::new(&genesis_info, None),
        BlockComponents::new_manifests(vec![]).as_terminal(),
        &params.runtime_params(),
    );
    (state, result)
}

fn v0_params() -> OLParams {
    OLParams::builder(OLRuntimeParams::test_default())
        .genesis_spec(OLSpecId::V0)
        .build()
}

#[test]
fn test_v0_genesis_matches_v1_genesis_with_bare_root() {
    let (v0_state, v0_genesis) = run_empty_genesis(OLSpecId::V0, &v0_params());
    let v0_genesis = v0_genesis.expect("V0 genesis executes");
    let (v1_state, v1_genesis) = run_empty_genesis(OLSpecId::V1, &OLParams::test_default());
    let v1_genesis = v1_genesis.expect("V1 genesis executes");

    // V0 rules leave the versions alone, so the root stays the bare
    // chainstate root that V0 headers commit to.
    assert_eq!(
        v0_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V0)
    );
    let chainstate_root = v0_state.chainstate().compute_chainstate_root();
    assert_eq!(*v0_genesis.header().state_root(), chainstate_root);
    assert_eq!(
        v0_state.compute_state_root().expect("state root"),
        chainstate_root
    );

    // Apart from the root form, the two genesis blocks are the same block.
    assert_eq!(
        v1_state.chainstate().compute_chainstate_root(),
        chainstate_root
    );
    assert_ne!(
        v1_genesis.header().state_root(),
        v0_genesis.header().state_root()
    );
    assert_eq!(
        &tamper_state_root(v1_genesis.header(), chainstate_root),
        v0_genesis.header()
    );
    assert_eq!(v1_genesis.body(), v0_genesis.body());
}

#[test]
fn test_v0_genesis_rejects_non_genesis_inputs() {
    let v0_params = v0_params();
    let genesis_with_pending_log = {
        let mut state = MemoryStateBaseLayer::new_genesis(&v0_params).expect("genesis state");
        let deposit = make_deposit_manifest_with_destination_bytes(
            1,
            0,
            vec![0xff],
            BitcoinAmount::try_from(1).expect("amount must not exceed the Bitcoin money supply"),
        );
        process_block_manifests(&mut state, &[deposit]).expect("buffer deposit log");
        state
    };
    let v0_genesis_state = || MemoryStateBaseLayer::new_genesis(&v0_params).expect("genesis state");
    let cases = [
        (
            "manifests in the genesis block",
            v0_genesis_state(),
            build_terminal_genesis_components(),
        ),
        (
            "transactions in the genesis block",
            v0_genesis_state(),
            BlockComponents::new_txs_from_ol_transactions(vec![make_gam_tx(make_account_id(
                TEST_RECIPIENT_ID,
            ))])
            .as_terminal(),
        ),
        (
            "a nonterminal genesis block",
            v0_genesis_state(),
            BlockComponents::new_manifests(vec![]),
        ),
        (
            "a buffered ASM log",
            genesis_with_pending_log,
            BlockComponents::new_manifests(vec![]).as_terminal(),
        ),
    ];

    for (case, mut state, components) in cases {
        let pre_root = state.compute_state_root().expect("state root");
        let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);
        let err = execute_and_complete_block(
            OLSpecId::V0,
            &mut state,
            BlockContext::new(&genesis_info, None),
            components,
            &v0_params.runtime_params(),
        )
        .expect_err(case);

        assert!(
            matches!(err, ExecError::UnimplementedSpec(OLSpecId::V0)),
            "{case}: {err}"
        );
        assert_eq!(
            state.compute_state_root().expect("state root"),
            pre_root,
            "{case}: a rejected block must not touch the state"
        );
    }
}

/// V0 rules never run on a state of a later spec, here a V1 genesis state.
#[test]
fn test_v0_genesis_rejects_later_spec_state() {
    let mut state = make_genesis_state();
    let pre_root = state.compute_state_root().expect("state root");
    let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);

    let err = execute_and_complete_block(
        OLSpecId::V0,
        &mut state,
        BlockContext::new(&genesis_info, None),
        BlockComponents::new_manifests(vec![]).as_terminal(),
        &OLRuntimeParams::test_default(),
    )
    .expect_err("V0 rules do not run on a V1 state");

    assert!(
        matches!(
            err,
            ExecError::StateFromLaterSpec {
                spec: OLSpecId::V0,
                state_spec_version: 1,
            }
        ),
        "{err}"
    );
    assert_eq!(state.compute_state_root().expect("state root"), pre_root);
}

#[test]
fn test_v0_genesis_block_on_post_genesis_state_fails_epoch_check() {
    let params = v0_params();
    let (mut state, genesis) = run_empty_genesis(OLSpecId::V0, &params);
    genesis.expect("V0 genesis executes");
    let post_genesis_root = state.compute_state_root().expect("state root");
    let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);

    let err = execute_and_complete_block(
        OLSpecId::V0,
        &mut state,
        BlockContext::new(&genesis_info, None),
        BlockComponents::new_manifests(vec![]).as_terminal(),
        &params.runtime_params(),
    )
    .expect_err("genesis runs only on the genesis state");

    assert!(
        matches!(err, ExecError::ContextEpochMismatch(0, 1)),
        "{err}"
    );
    assert_eq!(
        state.compute_state_root().expect("state root"),
        post_genesis_root,
        "a rejected block must not touch the state"
    );
}

/// V0 runs no block phase the sequencer, the mempool, DA rebuild or the guest
/// compose, and builds no block but genesis.
#[test]
fn test_v0_block_phases_are_unimplemented() {
    let (mut state, genesis) = run_empty_genesis(OLSpecId::V0, &v0_params());
    let genesis = genesis.expect("V0 genesis executes");
    let post_genesis_root = state.compute_state_root().expect("state root");
    let block_info = BlockInfo::new(
        EPOCH_RUNNER_GENESIS_TIMESTAMP + EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
        1,
        1,
    );
    let block_context = BlockContext::new(&block_info, Some(genesis.header()));
    let output = ExecOutputBuffer::new_empty();
    let runtime_params = OLRuntimeParams::test_default();
    let basic_ctx = BasicExecContext::new(block_info, &output, &runtime_params);
    let tx = make_gam_tx(make_account_id(TEST_RECIPIENT_ID));

    // V0 rejects the operation before reading its inputs, so the snark update
    // and its account can come from a V1 state.
    let mut sau_state = make_genesis_state();
    epoch_runner_seed_accounts(&mut sau_state);
    let sau_tx = build_snark_update(&sau_state, &snark_inbox_msg());
    let TransactionPayloadV1::SnarkAccountUpdate(sau) = sau_tx.data().payload() else {
        panic!("snark update payload");
    };
    let snark_account = sau_state
        .get_account_state(*sau.target())
        .expect("read snark account")
        .expect("snark account exists");
    let epoch_info = EpochInfo::new(block_info, genesis.header().compute_block_commitment());

    let results = [
        (
            "construct_block",
            construct_block(
                OLSpecId::V0,
                &mut state,
                block_context,
                BlockComponents::new_empty().as_terminal(),
                &runtime_params,
            )
            .map(|_| ()),
        ),
        (
            "verify_epoch_with_diff",
            verify_epoch_with_diff(
                OLSpecId::V0,
                &mut state,
                &epoch_info,
                &[],
                &[],
                &EpochExecExpectations::new(post_genesis_root),
                &runtime_params,
            )
            .map_err(|err| match err {
                EpochDaReplayError::Exec(err) => err,
                EpochDaReplayError::Decode(err) => panic!("decoded under V0: {err}"),
            }),
        ),
        (
            "predict_tx_log_payloads",
            sequencer::predict_tx_log_payloads(
                OLSpecId::V0,
                &tx,
                runtime_params.bridge_params(),
                |_| {},
            ),
        ),
        (
            "index_snark_update_proof_requirements",
            sequencer::index_snark_update_proof_requirements(
                OLSpecId::V0,
                *sau.target(),
                snark_account,
                sau.operation(),
                sau_tx.data().effects(),
            )
            .map(|_| ()),
        ),
        (
            "execute_block_initialization",
            sequencer::execute_block_initialization(OLSpecId::V0, &mut state, &block_context),
        ),
        (
            "process_single_tx",
            sequencer::process_single_tx(
                OLSpecId::V0,
                &mut state,
                &tx,
                &TxExecContext::new(&basic_ctx, Some(genesis.header())),
            ),
        ),
        (
            "check_tx_constraints",
            sequencer::check_tx_constraints(OLSpecId::V0, tx.constraints(), &state),
        ),
        (
            "process_asm_manifest",
            sequencer::process_asm_manifest(OLSpecId::V0, &mut state, &make_empty_manifest(1, 0))
                .map(|_| ()),
        ),
        (
            "process_epoch_terminal",
            sequencer::process_epoch_terminal(OLSpecId::V0, &mut state, &basic_ctx),
        ),
        (
            "verify_block_structure",
            sequencer::verify_block_structure(OLSpecId::V0, genesis.header(), genesis.body()),
        ),
        (
            "execute_block_batch_predrain",
            execute_block_batch_predrain(
                OLSpecId::V0,
                &mut state,
                &[],
                genesis.header(),
                &runtime_params,
            )
            .map(|_| ()),
        ),
    ];

    for (op, result) in results {
        assert!(
            matches!(result, Err(ExecError::UnimplementedSpec(OLSpecId::V0))),
            "{op}: {result:?}"
        );
    }
    assert_eq!(
        state.compute_state_root().expect("state root"),
        post_genesis_root,
        "a rejected phase must not touch the state"
    );
}

#[test]
fn test_v0_non_genesis_block_is_unimplemented() {
    let (mut state, genesis) = run_empty_genesis(OLSpecId::V0, &v0_params());
    let genesis = genesis.expect("V0 genesis executes");
    let post_genesis_root = state.compute_state_root().expect("state root");
    let block_info = BlockInfo::new(
        EPOCH_RUNNER_GENESIS_TIMESTAMP + EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
        1,
        1,
    );

    let err = execute_and_complete_block(
        OLSpecId::V0,
        &mut state,
        BlockContext::new(&block_info, Some(genesis.header())),
        BlockComponents::new_empty().as_terminal(),
        &OLRuntimeParams::test_default(),
    )
    .expect_err("V0 rules run only the genesis block");

    assert!(matches!(err, ExecError::UnimplementedSpec(OLSpecId::V0)));
    assert_eq!(
        state.compute_state_root().expect("state root"),
        post_genesis_root,
        "a rejected block must not touch the state"
    );
}
