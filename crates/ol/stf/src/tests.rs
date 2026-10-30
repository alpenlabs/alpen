//! Smoke tests for dispatch under [`OLSpecId::V1`], tests for the
//! [`OLSpecId::V0`] genesis and epoch replay, for the states [`OLSpecId::V2`]
//! takes over and the capacities it enforces, and for spec selection.
//!
//! The differential tests in `strata-ol-checkpoint` and
//! `strata-ol-block-assembly` compare the drivers against each other.

use std::convert::Infallible;
use std::{iter, slice};

use strata_acct_types::{AccountId, BitcoinAmount, Hash, MAX_MESSAGES, MAX_TRANSFERS};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::EePredicateKeyUpdate;
use strata_codec::encode_to_vec;
use strata_da_framework::{DaCounter, DaLinacc, DaRegister};
use strata_identifiers::{
    AccountSerial, BRIDGE_GATEWAY_ACCT_ID, L1BlockCommitment, OLBlockCommitment, SubjectId,
};
use strata_ol_chain_types_v1::{AsmManifest, MAX_LOGS_PER_BLOCK, OLBlockV1};
use strata_ol_da_common::{U16LenBytes, U16LenList};
use strata_ol_da_types_v1::{
    AccountDiffEntryV1, AccountDiffV1, DaProofStateDiffV1, GlobalStateDiffV1, LedgerDiffV1,
    OLDaPayloadV1, OLStateDiffV1, SnarkAccountDiffV1, decode_ol_da_payload_bytes,
};
use strata_ol_params::{OLParams, OLRuntimeParams};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{
    IAccountState, ISnarkAccountState, IStateAccessor, IStateAccessorMut, OLSpecVersions,
};
use strata_ol_state_types_v1::{OLSnarkAccountStateV1, OLStateV1};
use strata_ol_stf_v1::process_block_manifests;
use strata_ol_stf_v1::test_utils::*;
use strata_ol_tx_types_v1::{
    OLTransactionDataV1, OLTransactionV1, TransactionPayloadV1, TxProofsV1,
};
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{
    BasicExecContext, BlockComponents, BlockContext, BlockInfo, CompletedBlock,
    ConstructBlockOutput, EpochDaReplayError, EpochExecExpectations, EpochInfo, EpochL1Range,
    EpochSpecSelectionError, ExecError, ExecOutputBuffer, ExecResult, OLSpecId, OLSpecLimits,
    TxEffectsOverflow, TxExecContext, apply_da_epoch, construct_block, execute_and_complete_block,
    execute_block_batch_predrain, next_epoch_spec, select_next_epoch_spec, sequencer, spec_limits,
    verify_block, verify_epoch_with_diff,
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
fn test_v0_apply_da_epoch_rejects_v1_state() {
    let epoch = build_v1_epoch();
    let terminal = epoch.epoch_blocks.last().expect("epoch has a terminal");
    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal.header()),
        epoch.genesis.header().compute_block_commitment(),
    );
    let mut state = epoch.pre_epoch_state;
    let pre_root = state.compute_state_root().expect("state root");

    // V0 rules never take over a later state. The state is checked before the
    // diff is decoded, so even bytes that would not decode report it.
    let err = apply_da_epoch(
        OLSpecId::V0,
        &mut state,
        &epoch_info,
        &[0xff; 3],
        &[],
        &OLRuntimeParams::test_default(),
    )
    .expect_err("V0 does not replay a V1 state");

    assert!(matches!(
        err,
        EpochDaReplayError::Exec(ExecError::StateFromLaterSpec {
            spec: OLSpecId::V0,
            state_spec_version: 1,
        })
    ));
    assert_eq!(state.compute_state_root().expect("state root"), pre_root);
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

/// A V0 genesis state with the epoch runner's snark and empty accounts, past
/// its genesis block, with the commitment of that block.
fn v0_post_genesis_state() -> (MemoryStateBaseLayer<OLStateV1>, OLBlockCommitment) {
    let params = v0_params();
    let mut state = MemoryStateBaseLayer::new_genesis(&params).expect("genesis state");
    epoch_runner_seed_accounts(&mut state);
    let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);
    let genesis = execute_and_complete_block(
        OLSpecId::V0,
        &mut state,
        BlockContext::new(&genesis_info, None),
        BlockComponents::new_manifests(vec![]).as_terminal(),
        &params.runtime_params(),
    )
    .expect("V0 genesis executes");
    (state, genesis.header().compute_block_commitment())
}

/// Replays epoch 1 on `state` under `spec`, with an empty DA diff and
/// `manifests`.
fn replay_epoch_1(
    spec: OLSpecId,
    state: &mut MemoryStateBaseLayer<OLStateV1>,
    prev_terminal: OLBlockCommitment,
    manifests: &[AsmManifest],
) -> Result<(), EpochDaReplayError> {
    let diff = encode_to_vec(&OLDaPayloadV1::new(OLStateDiffV1::default())).expect("encode diff");
    replay_epoch_1_with_diff(spec, state, prev_terminal, &diff, manifests)
}

/// Replays epoch 1 on `state` under `spec`, with the encoded DA diff `diff`
/// and `manifests`.
fn replay_epoch_1_with_diff(
    spec: OLSpecId,
    state: &mut MemoryStateBaseLayer<OLStateV1>,
    prev_terminal: OLBlockCommitment,
    diff: &[u8],
    manifests: &[AsmManifest],
) -> Result<(), EpochDaReplayError> {
    let terminal_info = BlockInfo::new(
        EPOCH_RUNNER_GENESIS_TIMESTAMP + EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
        1,
        1,
    );
    apply_da_epoch(
        spec,
        state,
        &EpochInfo::new(terminal_info, prev_terminal),
        diff,
        manifests,
        &OLRuntimeParams::test_default(),
    )
}

fn ee_key_update_manifest(height: u32, updates: &[(AccountSerial, PredicateKey)]) -> AsmManifest {
    let logs = updates.iter().map(|(serial, key)| {
        AsmLogEntry::from_log(&EePredicateKeyUpdate::new(*serial, key.clone()))
            .expect("key update log encodes")
    });
    FixtureAsmManifestBuilder::new_at_height(height)
        .with_logs(logs)
        .build()
}

fn snark_account(state: &MemoryStateBaseLayer<OLStateV1>) -> OLSnarkAccountStateV1 {
    get_snark_state_expect(state, make_account_id(TEST_SNARK_ACCOUNT_ID))
        .1
        .clone()
}

fn account_serial(state: &MemoryStateBaseLayer<OLStateV1>, index: u32) -> AccountSerial {
    state
        .get_account_state(make_account_id(index))
        .expect("read account")
        .expect("account exists")
        .serial()
}

/// The V0 drain sets the EE key at once and skips updates for an unknown
/// serial or a non-snark account; V1 queues the key in the inbox instead.
#[test]
fn test_v0_ee_key_update_applies_at_once() {
    let (state, genesis) = v0_post_genesis_state();
    let new_key = PredicateKey::try_new(PredicateTypeId::AlwaysAccept, b"rotated".to_vec())
        .expect("predicate condition fits");
    let manifest = ee_key_update_manifest(
        state.last_l1_height() + 1,
        &[
            (AccountSerial::from(9_999), new_key.clone()),
            (account_serial(&state, TEST_RECIPIENT_ID), new_key.clone()),
            (
                account_serial(&state, TEST_SNARK_ACCOUNT_ID),
                new_key.clone(),
            ),
        ],
    );
    let before = snark_account(&state);

    let mut v0_state = state.clone();
    replay_epoch_1(
        OLSpecId::V0,
        &mut v0_state,
        genesis,
        slice::from_ref(&manifest),
    )
    .expect("V0 replay");
    let v0_snark = snark_account(&v0_state);
    assert_eq!(v0_snark.update_vk(), &new_key);
    assert_eq!(
        v0_snark.inbox_mmr().num_entries(),
        before.inbox_mmr().num_entries()
    );
    assert!(
        v0_state
            .get_account_state(make_account_id(TEST_RECIPIENT_ID))
            .expect("read empty account")
            .expect("empty account exists")
            .as_snark_account()
            .is_err()
    );
    assert_eq!(
        v0_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V0)
    );
    assert_eq!(
        v0_state.compute_state_root().expect("state root"),
        v0_state.chainstate().compute_chainstate_root()
    );

    let mut v1_state = state;
    replay_epoch_1(OLSpecId::V1, &mut v1_state, genesis, &[manifest]).expect("V1 replay");
    let v1_snark = snark_account(&v1_state);
    assert_eq!(v1_snark.update_vk(), before.update_vk());
    assert_eq!(
        v1_snark.inbox_mmr().num_entries(),
        before.inbox_mmr().num_entries() + 1
    );
}

/// V0 replay ignores checkpoint predicate enactments and does not check where
/// they sit; V1 rejects an enactment before the last manifest.
#[test]
fn test_v0_replay_ignores_enactments() {
    let (state, genesis) = v0_post_genesis_state();
    let next_height = state.last_l1_height() + 1;
    let manifests = [
        make_checkpoint_predicate_enactment_manifest(next_height, 1),
        make_empty_manifest(next_height + 1, 0),
    ];

    let mut v0_state = state.clone();
    replay_epoch_1(OLSpecId::V0, &mut v0_state, genesis, &manifests).expect("V0 replay");
    assert_eq!(v0_state.last_l1_height(), next_height + 1);
    assert_eq!(
        v0_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V0)
    );

    let mut v1_state = state;
    let err = replay_epoch_1(OLSpecId::V1, &mut v1_state, genesis, &manifests)
        .expect_err("V1 rejects an enactment before the last manifest");
    assert!(
        matches!(
            err,
            EpochDaReplayError::Exec(ExecError::CheckpointPredicateBoundaryNotLast { .. })
        ),
        "{err}"
    );
}

/// Encodes a DA payload whose only change sets the snark account's inner
/// state, and `update_vk` if given.
fn snark_inner_state_payload(
    state: &MemoryStateBaseLayer<OLStateV1>,
    update_vk: Option<Vec<u8>>,
) -> Vec<u8> {
    let snark_diff = SnarkAccountDiffV1::new(
        DaCounter::new_unchanged(),
        DaProofStateDiffV1::new(
            DaRegister::new_set(Hash::from([0x22; 32])),
            DaCounter::new_unchanged(),
        ),
        DaLinacc::new(),
        DaRegister::new(update_vk.map(U16LenBytes::new)),
    );
    let account_diff = AccountDiffEntryV1::new(
        account_serial(state, TEST_SNARK_ACCOUNT_ID),
        AccountDiffV1::new(DaCounter::new_unchanged(), snark_diff),
    );
    let diff = OLStateDiffV1::new(
        GlobalStateDiffV1::default(),
        LedgerDiffV1::new(
            U16LenList::new(Vec::new()),
            U16LenList::new(vec![account_diff]),
        ),
    );
    encode_to_vec(&OLDaPayloadV1::new(diff)).expect("encode diff")
}

/// V0 reads a snark account diff from its first three presence bits and
/// ignores the rest, and V0 checkpoints prove the DA bytes as read that way.
/// So a payload that also sets the `update_vk` bit replays as the canonical
/// one under V0, where the V1 decoder rejects it.
#[test]
fn test_v0_replay_ignores_snark_bits_past_v0_members() {
    let (state, genesis) = v0_post_genesis_state();
    let canonical = snark_inner_state_payload(&state, None);
    let with_update_vk = snark_inner_state_payload(&state, Some(vec![0x42; 33]));
    // The two encodings first differ in the snark diff's presence bitmap.
    let bitmap_offset = canonical
        .iter()
        .zip(&with_update_vk)
        .position(|(canonical, with_update_vk)| canonical != with_update_vk)
        .expect("the encodings differ");
    let mut flagged = canonical.clone();
    flagged[bitmap_offset] |= 0b1000;
    assert_eq!(flagged[bitmap_offset], with_update_vk[bitmap_offset]);
    assert!(decode_ol_da_payload_bytes(&flagged).is_err());

    let mut canonical_state = state.clone();
    replay_epoch_1_with_diff(OLSpecId::V0, &mut canonical_state, genesis, &canonical, &[])
        .expect("canonical payload replays");
    let mut flagged_state = state.clone();
    replay_epoch_1_with_diff(OLSpecId::V0, &mut flagged_state, genesis, &flagged, &[])
        .expect("payload with an ignored bit replays");

    let canonical_root = canonical_state.compute_state_root().expect("state root");
    assert_eq!(
        flagged_state.compute_state_root().expect("state root"),
        canonical_root
    );
    assert_ne!(
        canonical_root,
        state.compute_state_root().expect("state root"),
        "the payload changes the state"
    );
    assert_eq!(
        snark_account(&flagged_state).update_vk(),
        snark_account(&state).update_vk()
    );
}

/// Deposits V0 sweeps to limbo and the log types it ignores leave the
/// chainstate V1 leaves, whose deposit handler V0 shares; only the root form
/// differs.
#[test]
fn test_v0_replay_limbos_deposits_and_ignores_logs_like_v1() {
    let (state, genesis) = v0_post_genesis_state();
    let height = state.last_l1_height() + 1;
    let amount = BitcoinAmount::try_from(150_000_000)
        .expect("amount must not exceed the Bitcoin money supply");
    let malformed_destination =
        make_deposit_manifest_with_destination_bytes(height, 0, vec![0xff], amount).logs()[0]
            .clone();
    let unknown_serial = make_deposit_log_for_account(
        AccountSerial::from(9_999),
        SubjectId::from([0xee; 32]),
        amount,
    );
    let mut logs = vec![malformed_destination, unknown_serial];
    for ty in [2, 11, 12, 13] {
        logs.push(AsmLogEntry::from_msg(ty, vec![0xab; 8]).expect("log encodes"));
    }
    let manifest = FixtureAsmManifestBuilder::new_at_height(height)
        .with_logs(logs)
        .build();

    let mut v0_state = state.clone();
    replay_epoch_1(
        OLSpecId::V0,
        &mut v0_state,
        genesis,
        slice::from_ref(&manifest),
    )
    .expect("V0 replay");
    assert_eq!(
        v0_state.limbo_funds().to_sat(),
        state.limbo_funds().to_sat() + 2 * amount.to_sat()
    );

    let mut v1_state = state;
    replay_epoch_1(OLSpecId::V1, &mut v1_state, genesis, &[manifest]).expect("V1 replay");
    assert_eq!(
        v0_state.chainstate().compute_chainstate_root(),
        v1_state.chainstate().compute_chainstate_root()
    );
}

#[test]
fn test_v0_replay_rejects_manifest_height_gap() {
    let (mut state, genesis) = v0_post_genesis_state();
    let gap_height = state.last_l1_height() + 2;

    let err = replay_epoch_1(
        OLSpecId::V0,
        &mut state,
        genesis,
        &[make_empty_manifest(gap_height, 0)],
    )
    .expect_err("manifest heights must follow on");

    assert!(
        matches!(
            err,
            EpochDaReplayError::Exec(ExecError::AsmManifestHeightMismatch { .. })
        ),
        "{err}"
    );
}

/// V2 takes over a V1 state when it opens an epoch, as the first V2 epoch
/// does after a V1 enactment, but never continues an epoch that started
/// under V1.
#[test]
fn test_v2_takes_over_a_v1_state_only_at_an_epoch_start() {
    let epoch = build_v1_epoch();
    let runtime_params = OLRuntimeParams::test_default();
    let [first, second, ..] = epoch.epoch_blocks.as_slice() else {
        panic!("the epoch has at least two blocks");
    };

    // The sequencer opens a block through block initialization, which takes
    // over the state at the start of an epoch.
    let block_info = BlockInfo::from_header(first.header());
    let block_context = BlockContext::new(&block_info, Some(epoch.genesis.header()));
    let mut state = epoch.pre_epoch_state.clone();
    sequencer::execute_block_initialization(OLSpecId::V2, &mut state, &block_context)
        .expect("V2 opens the epoch");
    assert_eq!(state.spec_versions(), OLSpecVersions::uniform(OLSpecId::V2));

    // A V1 state in the middle of an epoch, after V1 ran the epoch's first
    // block.
    let mut state = epoch.pre_epoch_state;
    verify_block(
        OLSpecId::V1,
        &mut state,
        first.header(),
        Some(epoch.genesis.header()),
        first.body(),
        &runtime_params,
    )
    .expect("V1 verifies its own block");
    let mid_epoch_root = state.compute_state_root().expect("state root");

    let err = verify_block(
        OLSpecId::V2,
        &mut state,
        second.header(),
        Some(first.header()),
        second.body(),
        &runtime_params,
    )
    .expect_err("V2 must not continue a V1 epoch");
    assert!(
        matches!(
            err,
            ExecError::ContinuesEarlierSpecEpoch {
                spec: OLSpecId::V2,
                state_spec: OLSpecId::V1,
            }
        ),
        "{err}"
    );

    let err = execute_block_batch_predrain(
        OLSpecId::V2,
        &mut state,
        slice::from_ref(second),
        first.header(),
        &runtime_params,
    )
    .expect_err("V2 must not continue a V1 epoch in a batch");
    assert!(
        matches!(err, ExecError::ContinuesEarlierSpecEpoch { .. }),
        "{err}"
    );

    let block_info = BlockInfo::from_header(second.header());
    let block_context = BlockContext::new(&block_info, Some(first.header()));
    let err = sequencer::execute_block_initialization(OLSpecId::V2, &mut state, &block_context)
        .expect_err("V2 must not open a block that continues a V1 epoch");
    assert!(
        matches!(err, ExecError::ContinuesEarlierSpecEpoch { .. }),
        "{err}"
    );

    assert_eq!(
        state.compute_state_root().expect("state root"),
        mid_epoch_root,
        "a rejected takeover must not touch the state"
    );
}

/// Each enactment advances the spec by one, so V2 never takes over a V0
/// state.
#[test]
fn test_v2_rejects_a_v0_state() {
    let epoch = build_v1_epoch();
    let runtime_params = OLRuntimeParams::test_default();
    let first = &epoch.epoch_blocks[0];
    let terminal = epoch.epoch_blocks.last().expect("epoch has a terminal");
    let mut state = epoch.pre_epoch_state;
    state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V0));
    let pre_root = state.compute_state_root().expect("state root");

    let err = verify_block(
        OLSpecId::V2,
        &mut state,
        first.header(),
        Some(epoch.genesis.header()),
        first.body(),
        &runtime_params,
    )
    .expect_err("V2 must not take over a V0 state");
    assert!(
        matches!(
            err,
            ExecError::StateFromEarlierSpec {
                spec: OLSpecId::V2,
                state_spec: OLSpecId::V0,
            }
        ),
        "{err}"
    );

    // Replay checks the state before it decodes the diff, so even bytes that
    // would not decode report it.
    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal.header()),
        epoch.genesis.header().compute_block_commitment(),
    );
    let err = apply_da_epoch(
        OLSpecId::V2,
        &mut state,
        &epoch_info,
        &[0xff; 3],
        &[],
        &runtime_params,
    )
    .expect_err("V2 must not replay an epoch on a V0 state");
    assert!(
        matches!(
            err,
            EpochDaReplayError::Exec(ExecError::StateFromEarlierSpec { .. })
        ),
        "{err}"
    );

    assert_eq!(
        state.compute_state_root().expect("state root"),
        pre_root,
        "a rejected takeover must not touch the state"
    );
}

/// Returns the commitment of the L1 block `manifest` is for.
fn manifest_block(manifest: &AsmManifest) -> L1BlockCommitment {
    L1BlockCommitment::new(manifest.height(), *manifest.blkid())
}

/// Selects the spec after a parent epoch whose terminal state runs `spec` and
/// which processed the L1 blocks after `prev_epoch_last_l1_block` up to `last`, the block of
/// `manifest`.
fn select_after(
    spec: OLSpecId,
    prev_epoch_last_l1_block: L1BlockCommitment,
    manifest: &AsmManifest,
) -> Result<OLSpecId, EpochSpecSelectionError<Infallible>> {
    let range = EpochL1Range::new(prev_epoch_last_l1_block, manifest_block(manifest))
        .expect("ordered range");
    select_next_epoch_spec(OLSpecVersions::uniform(spec), range, |block| {
        assert_eq!(*block, manifest_block(manifest));
        Ok(Some(manifest.clone()))
    })
}

#[test]
fn test_next_epoch_spec_advances_only_at_an_enactment() {
    assert_eq!(next_epoch_spec(OLSpecId::V0, None), Ok(OLSpecId::V0));
    assert_eq!(next_epoch_spec(OLSpecId::V0, Some(10)), Ok(OLSpecId::V1));
    assert_eq!(next_epoch_spec(OLSpecId::V1, None), Ok(OLSpecId::V1));
    assert_eq!(next_epoch_spec(OLSpecId::V1, Some(10)), Ok(OLSpecId::V2));
    assert_eq!(next_epoch_spec(OLSpecId::V2, None), Ok(OLSpecId::V2));

    // This binary implements no spec after V2.
    let err = next_epoch_spec(OLSpecId::V2, Some(10)).expect_err("V2 has no known successor");
    assert_eq!(err.prev_spec(), OLSpecId::V2);
    assert_eq!(err.enactment_l1_height(), 10);
    assert_eq!(err.spec_version(), 3);
}

#[test]
fn test_select_switches_at_an_enactment_the_parent_processed() {
    let prev_epoch_last_l1_block = manifest_block(&make_empty_manifest(9, 0));
    let enactment = make_checkpoint_predicate_enactment_manifest(10, 1);

    for (spec, successor) in [(OLSpecId::V0, OLSpecId::V1), (OLSpecId::V1, OLSpecId::V2)] {
        assert_eq!(
            select_after(spec, prev_epoch_last_l1_block, &enactment).expect("has a successor"),
            successor
        );
    }

    // V2 -> V3: the epoch after a V2 epoch that processed an enactment runs
    // V3, which this binary does not implement.
    let err = select_after(OLSpecId::V2, prev_epoch_last_l1_block, &enactment)
        .expect_err("upgrade required");
    let EpochSpecSelectionError::UpgradeRequired(upgrade) = err else {
        panic!("expected an upgrade-required error, got {err}");
    };
    assert_eq!(upgrade.prev_spec(), OLSpecId::V2);
    assert_eq!(upgrade.enactment_l1_height(), 10);
    assert_eq!(upgrade.spec_version(), 3);
}

#[test]
fn test_select_keeps_the_spec_after_a_plain_last_manifest() {
    let prev_epoch_last_l1_block = manifest_block(&make_empty_manifest(9, 0));
    let plain = make_empty_manifest(12, 1);
    for spec in [OLSpecId::V0, OLSpecId::V1, OLSpecId::V2] {
        assert_eq!(
            select_after(spec, prev_epoch_last_l1_block, &plain).expect("selects"),
            spec
        );
    }
}

/// An epoch that processes no manifests keeps the previous epoch's last L1
/// block. The first V1 epoch can therefore still end on `B`, the block whose
/// enactment started V1; that enactment must not select V2 for the second
/// V1 epoch.
#[test]
fn test_select_ignores_an_enactment_the_parent_did_not_process() {
    let b = manifest_block(&make_checkpoint_predicate_enactment_manifest(10, 1));
    for spec in [OLSpecId::V0, OLSpecId::V1, OLSpecId::V2] {
        let selected = select_next_epoch_spec(
            OLSpecVersions::uniform(spec),
            EpochL1Range::empty(b),
            |_| -> Result<Option<AsmManifest>, Infallible> {
                panic!("an empty range reads no manifest")
            },
        )
        .expect("selects");
        assert_eq!(selected, spec);
    }
}

#[test]
fn test_select_never_reads_the_staged_version() {
    let prev_epoch_last_l1_block = manifest_block(&make_empty_manifest(9, 0));
    let plain = make_empty_manifest(10, 1);
    let range =
        EpochL1Range::new(prev_epoch_last_l1_block, manifest_block(&plain)).expect("ordered range");
    let versions = OLSpecVersions::new(OLSpecId::V1, 7).expect("V1 may stage any version");
    let selected = select_next_epoch_spec(versions, range, |_| {
        Ok::<_, Infallible>(Some(plain.clone()))
    })
    .expect("selects");
    assert_eq!(selected, OLSpecId::V1);
}

#[test]
fn test_select_rejects_a_missing_or_foreign_last_manifest() {
    let prev_epoch_last_l1_block = manifest_block(&make_empty_manifest(9, 0));
    let last = manifest_block(&make_empty_manifest(10, 1));
    let range = EpochL1Range::new(prev_epoch_last_l1_block, last).expect("ordered range");
    let v1 = OLSpecVersions::uniform(OLSpecId::V1);

    let err = select_next_epoch_spec(v1, range, |_| Ok::<_, Infallible>(None))
        .expect_err("missing manifest");
    assert!(
        matches!(err, EpochSpecSelectionError::MissingLastManifest { block } if block == last),
        "{err}"
    );

    // Another block at the same height, as after an L1 reorg, and the right
    // block ID at another height.
    for foreign in [make_empty_manifest(10, 2), make_empty_manifest(11, 1)] {
        let found = manifest_block(&foreign);
        let err = select_next_epoch_spec(v1, range, |_| Ok::<_, Infallible>(Some(foreign.clone())))
            .expect_err("foreign manifest");
        assert!(
            matches!(
                err,
                EpochSpecSelectionError::LastManifestMismatch { expected, found: got }
                    if expected == last && got == found
            ),
            "{err}"
        );
    }

    let err = select_next_epoch_spec(v1, range, |_| Err("db down")).expect_err("lookup fails");
    assert!(
        matches!(err, EpochSpecSelectionError::ManifestLookup { block, source: "db down" } if block == last),
        "{err}"
    );
}

#[test]
fn test_select_rejects_a_duplicated_enactment() {
    let prev_epoch_last_l1_block = manifest_block(&make_empty_manifest(9, 0));
    let duplicate = make_checkpoint_predicate_enactment_manifest(10, 2);
    let err =
        select_after(OLSpecId::V0, prev_epoch_last_l1_block, &duplicate).expect_err("duplicate");
    assert!(
        matches!(
            err,
            EpochSpecSelectionError::Exec(ExecError::DuplicateCheckpointPredicateEnactment {
                height: 10
            })
        ),
        "{err}"
    );
}

#[test]
fn test_epoch_l1_range_must_follow_the_previous_epoch() {
    let at_9 = manifest_block(&make_empty_manifest(9, 0));
    let at_10 = manifest_block(&make_empty_manifest(10, 0));
    let other_at_10 = manifest_block(&make_empty_manifest(10, 1));

    assert!(EpochL1Range::new(at_10, at_9).is_err());
    assert!(EpochL1Range::new(at_10, other_at_10).is_err());

    let empty = EpochL1Range::new(at_10, at_10).expect("an empty range");
    assert_eq!(empty, EpochL1Range::empty(at_10));
    assert_eq!(empty.last_processed(), None);

    let range = EpochL1Range::new(at_9, at_10).expect("an ordered range");
    assert_eq!(range.prev_last(), &at_9);
    assert_eq!(range.last(), &at_10);
    assert_eq!(range.last_processed(), Some(&at_10));
}

/// Satoshis each test withdrawal sends to the bridge gateway.
const WITHDRAWAL_SATS: u64 = 100_000_000;

/// A post-genesis state with a funded snark account, and transactions built
/// against it for the first block of epoch 1.
struct CapacityFixture {
    fixture: OLStfFixture,
    sender: AccountId,
    recipient: AccountId,
}

impl CapacityFixture {
    /// Funds the snark account for `withdrawal_count` withdrawals.
    fn new(withdrawal_count: usize) -> Self {
        let sender = make_account_id(TEST_SNARK_ACCOUNT_ID);
        let recipient = make_account_id(TEST_RECIPIENT_ID);
        let balance = BitcoinAmount::try_from(WITHDRAWAL_SATS * withdrawal_count as u64)
            .expect("balance fits the money supply");
        let mut fixture = OLStfFixture::builder()
            .with_genesis_snark_account(sender, |account| account.with_balance(balance))
            .execute_genesis();
        fixture.insert_empty_account(recipient);
        Self {
            fixture,
            sender,
            recipient,
        }
    }

    /// Builds one snark account update per entry of `effect_counts`, in
    /// sequence from `first_seq_no`, each sending that many
    /// `(transfers, withdrawals)`.
    fn updates(&self, first_seq_no: u64, effect_counts: &[(usize, usize)]) -> Vec<OLTransactionV1> {
        let account = self.fixture.expect_snark_account(self.sender);
        effect_counts
            .iter()
            .zip(first_seq_no..)
            .map(|(&(transfers, withdrawals), seq_no)| {
                let mut update =
                    SnarkUpdateBuilder::from_snark_state(account.clone()).with_seq_no(seq_no);
                for _ in 0..transfers {
                    update = update.with_transfer(self.recipient, 1);
                }
                for _ in 0..withdrawals {
                    update = update.with_output_message(
                        BRIDGE_GATEWAY_ACCT_ID,
                        WITHDRAWAL_SATS,
                        make_withdrawal_payload(make_p2wpkh_bosd_descriptor(0x15)),
                    );
                }
                update.build(
                    self.sender,
                    make_state_root((seq_no as u8).wrapping_add(2)),
                    make_proof(1),
                )
            })
            .collect()
    }

    /// Returns updates, in sequence from `first_seq_no`, that emit `log_count`
    /// logs between them: one per update and one per withdrawal, with at most
    /// 255 withdrawals each.
    fn updates_emitting(&self, first_seq_no: u64, log_count: usize) -> Vec<OLTransactionV1> {
        let max_logs_per_update = OLSpecLimits::V2.max_tx_messages() + 1;
        let effect_counts: Vec<_> = (0..log_count.div_ceil(max_logs_per_update))
            .map(|index| {
                let logs = (log_count - index * max_logs_per_update).min(max_logs_per_update);
                (0, logs - 1)
            })
            .collect();
        self.updates(first_seq_no, &effect_counts)
    }

    fn block_info(&self) -> BlockInfo {
        BlockInfo::new(1_001_000, 1, 1)
    }

    /// Constructs the first block of epoch 1 from `txs` under `spec`.
    fn construct(
        &self,
        spec: OLSpecId,
        txs: Vec<OLTransactionV1>,
    ) -> ExecResult<ConstructBlockOutput> {
        let block_info = self.block_info();
        construct_block(
            spec,
            &mut self.fixture.state().clone(),
            BlockContext::new(&block_info, Some(self.fixture.parent_header())),
            BlockComponents::new_txs_from_ol_transactions(txs),
            &OLRuntimeParams::test_default(),
        )
    }

    /// Builds a block from `txs` with the V1 code on a V2 state, so the block
    /// differs from a valid V2 block only in what V2 caps, and returns it with
    /// the state to verify it on.
    fn v2_block_built_by_v1(
        &self,
        txs: Vec<OLTransactionV1>,
    ) -> (OLBlockV1, MemoryStateBaseLayer<OLStateV1>) {
        let (mut blocks, pre_state) = self.v2_blocks_built_by_v1(vec![txs]);
        (blocks.remove(0), pre_state)
    }

    /// Builds consecutive blocks of epoch 1, one from each entry of
    /// `block_txs`, as [`Self::v2_block_built_by_v1`] builds one.
    fn v2_blocks_built_by_v1(
        &self,
        block_txs: Vec<Vec<OLTransactionV1>>,
    ) -> (Vec<OLBlockV1>, MemoryStateBaseLayer<OLStateV1>) {
        let mut state = self.fixture.state().clone();
        state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V2));
        let pre_state = state.clone();
        let mut parent = self.fixture.parent_header().clone();
        let mut blocks = Vec::new();
        for (txs, slot) in block_txs.into_iter().zip(1..) {
            let block_info = BlockInfo::new(1_000_000 + slot * 1_000, slot, 1);
            let output = strata_ol_stf_v1::construct_block(
                &mut state,
                BlockContext::new(&block_info, Some(&parent)),
                BlockComponents::new_txs_from_ol_transactions(txs),
                &OLRuntimeParams::test_default(),
            )
            .expect("the V1 code builds the block");
            parent = output.completed_block().header().clone();
            blocks.push(to_ol_block(output.completed_block()));
        }
        (blocks, pre_state)
    }

    /// Runs every V2 operation that executes a whole block on `block`.
    fn v2_block_errors(
        &self,
        block: &OLBlockV1,
        pre_state: &MemoryStateBaseLayer<OLStateV1>,
    ) -> Vec<(&'static str, ExecError)> {
        let parent = self.fixture.parent_header();
        let runtime_params = OLRuntimeParams::test_default();
        let block_info = BlockInfo::from_header(block.header());
        vec![
            (
                "verify_block",
                verify_block(
                    OLSpecId::V2,
                    &mut pre_state.clone(),
                    block.header(),
                    Some(parent),
                    block.body(),
                    &runtime_params,
                )
                .expect_err("verify_block rejects the block"),
            ),
            (
                "execute_and_complete_block",
                execute_and_complete_block(
                    OLSpecId::V2,
                    &mut pre_state.clone(),
                    BlockContext::new(&block_info, Some(parent)),
                    BlockComponents::from_block(block),
                    &runtime_params,
                )
                .expect_err("execute_and_complete_block rejects the block"),
            ),
            (
                "execute_block_batch_predrain",
                execute_block_batch_predrain(
                    OLSpecId::V2,
                    &mut pre_state.clone(),
                    slice::from_ref(block),
                    parent,
                    &runtime_params,
                )
                .expect_err("execute_block_batch_predrain rejects the block"),
            ),
        ]
    }
}

#[test]
fn test_spec_limits() {
    assert_eq!(spec_limits(OLSpecId::V1), OLSpecLimits::V1);
    assert_eq!(OLSpecLimits::V1.max_tx_transfers(), MAX_TRANSFERS as usize);
    assert_eq!(OLSpecLimits::V1.max_tx_messages(), MAX_MESSAGES as usize);
    assert_eq!(
        OLSpecLimits::V1.max_block_logs(),
        MAX_LOGS_PER_BLOCK as usize
    );

    let v2 = spec_limits(OLSpecId::V2);
    assert_eq!(v2, spec_limits(OLSpecId::V0));
    assert_eq!(v2.max_tx_transfers(), 255);
    assert_eq!(v2.max_tx_messages(), 255);
    assert_eq!(v2.max_block_logs(), 4_096);
}

/// V2 accepts a block whose logs reach its capacity and rejects one more log
/// in every operation that executes a whole block. V1 accepts both.
#[test]
fn test_v2_caps_block_logs() {
    let max_logs = OLSpecLimits::V2.max_block_logs();
    let capacity = CapacityFixture::new(max_logs);

    let output = capacity
        .construct(OLSpecId::V2, capacity.updates_emitting(0, max_logs))
        .expect("V2 builds a block at its log capacity");
    assert_eq!(output.outputs().logs().len(), max_logs);
    let block = output.completed_block();
    let logs = verify_block(
        OLSpecId::V2,
        &mut capacity.fixture.state().clone(),
        block.header(),
        Some(capacity.fixture.parent_header()),
        block.body(),
        &OLRuntimeParams::test_default(),
    )
    .expect("V2 verifies a block at its log capacity");
    assert_eq!(logs.len(), max_logs);

    let over = capacity.updates_emitting(0, max_logs + 1);
    let output = capacity
        .construct(OLSpecId::V1, over.clone())
        .expect("V1 accepts the extra log");
    assert_eq!(output.outputs().logs().len(), max_logs + 1);

    let is_overflow = |err: &ExecError| {
        matches!(err.base(), ExecError::LogsOverflow { count, max }
            if *count == max_logs + 1 && *max == max_logs)
    };
    let err = capacity
        .construct(OLSpecId::V2, over.clone())
        .expect_err("V2 rejects the extra log");
    assert!(is_overflow(&err), "{err}");

    let (block, pre_state) = capacity.v2_block_built_by_v1(over);
    for (operation, err) in capacity.v2_block_errors(&block, &pre_state) {
        assert!(is_overflow(&err), "{operation}: {err}");
    }
}

/// V2 accepts a transaction whose transfers and messages reach its
/// capacities and rejects one more of either, in block execution and in the
/// sequencer phases that take a single transaction. V1 accepts both.
#[test]
fn test_v2_caps_transaction_effects() {
    let max_transfers = OLSpecLimits::V2.max_tx_transfers();
    let max_messages = OLSpecLimits::V2.max_tx_messages();
    let capacity = CapacityFixture::new(max_messages + 1);

    capacity
        .construct(
            OLSpecId::V2,
            capacity.updates(0, &[(max_transfers, max_messages)]),
        )
        .expect("V2 accepts a transaction at its effect capacities");

    let runtime_params = OLRuntimeParams::test_default();
    let block_info = capacity.block_info();
    let output = ExecOutputBuffer::new_empty();
    let basic_ctx = BasicExecContext::new(block_info, &output, &runtime_params);
    let parent = capacity.fixture.parent_header();
    let sender_state = capacity
        .fixture
        .state()
        .get_account_state(capacity.sender)
        .expect("read snark account")
        .expect("snark account exists")
        .clone();

    for (effect_counts, expected) in [
        (
            (max_transfers + 1, 0),
            TxEffectsOverflow::Transfers {
                count: max_transfers + 1,
                max: max_transfers,
            },
        ),
        (
            (0, max_messages + 1),
            TxEffectsOverflow::Messages {
                count: max_messages + 1,
                max: max_messages,
            },
        ),
    ] {
        let txs = capacity.updates(0, &[effect_counts]);
        capacity
            .construct(OLSpecId::V1, txs.clone())
            .expect("V1 accepts the extra effect");

        let is_overflow = |err: &ExecError| matches!(err.base(), ExecError::TxEffectsOverflow(err) if *err == expected);
        let err = capacity
            .construct(OLSpecId::V2, txs.clone())
            .expect_err("V2 rejects the extra effect");
        assert!(is_overflow(&err), "{err}");
        assert!(
            matches!(err, ExecError::TxExec(_, 0, _)),
            "block execution names the transaction: {err}"
        );

        let (block, pre_state) = capacity.v2_block_built_by_v1(txs.clone());
        for (operation, err) in capacity.v2_block_errors(&block, &pre_state) {
            assert!(is_overflow(&err), "{operation}: {err}");
        }

        let tx = &txs[0];
        let TransactionPayloadV1::SnarkAccountUpdate(sau) = tx.data().payload() else {
            panic!("snark update payload");
        };
        let err = sequencer::index_snark_update_proof_requirements(
            OLSpecId::V2,
            capacity.sender,
            &sender_state,
            sau.operation(),
            tx.data().effects(),
        )
        .expect_err("proof indexing rejects the extra effect");
        assert!(is_overflow(&err), "{err}");

        let err = sequencer::process_single_tx(
            OLSpecId::V2,
            &mut capacity.fixture.state().clone(),
            tx,
            &TxExecContext::new(&basic_ctx, Some(parent)),
        )
        .expect_err("transaction processing rejects the extra effect");
        assert!(is_overflow(&err), "{err}");
    }
}

/// Batch pre-drain execution caps each block's logs, not the batch's: two
/// blocks at the cap run as the V1 code runs them, and a second block over
/// the cap fails after a first block that fits.
#[test]
fn test_v2_batch_predrain_caps_each_block() {
    let max_logs = OLSpecLimits::V2.max_block_logs();
    let capacity = CapacityFixture::new(3 * max_logs);
    let runtime_params = OLRuntimeParams::test_default();
    let parent = capacity.fixture.parent_header();
    let first = capacity.updates_emitting(0, max_logs);
    let second_seq_no = first.len() as u64;

    let (blocks, pre_state) = capacity.v2_blocks_built_by_v1(vec![
        first.clone(),
        capacity.updates_emitting(second_seq_no, max_logs),
    ]);
    let mut v2_state = pre_state.clone();
    let v2_logs = execute_block_batch_predrain(
        OLSpecId::V2,
        &mut v2_state,
        &blocks,
        parent,
        &runtime_params,
    )
    .expect("V2 runs two blocks at its log capacity");
    let mut v1_state = pre_state;
    let v1_logs = strata_ol_stf_v1::execute_block_batch_predrain(
        &mut v1_state,
        &blocks,
        parent,
        &runtime_params,
    )
    .expect("the V1 code runs the batch");
    assert_eq!(v2_logs.len(), 2 * max_logs);
    assert_eq!(v2_logs, v1_logs);
    assert_eq!(
        v2_state.compute_state_root().expect("state root"),
        v1_state.compute_state_root().expect("state root"),
    );

    let (blocks, pre_state) = capacity.v2_blocks_built_by_v1(vec![
        first,
        capacity.updates_emitting(second_seq_no, max_logs + 1),
    ]);
    let err = execute_block_batch_predrain(
        OLSpecId::V2,
        &mut pre_state.clone(),
        &blocks,
        parent,
        &runtime_params,
    )
    .expect_err("V2 rejects the second block");
    assert!(
        matches!(err.base(), ExecError::LogsOverflow { count, max }
            if *count == max_logs + 1 && *max == max_logs),
        "{err}"
    );
}
