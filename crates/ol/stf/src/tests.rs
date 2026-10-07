//! Smoke tests for dispatch under [`OLSpecId::V1`], tests for the
//! [`OLSpecId::V0`] genesis and epoch replay, and for spec selection.
//!
//! The differential tests in `strata-ol-checkpoint` and
//! `strata-ol-block-assembly` compare the drivers against each other.

use std::convert::Infallible;
use std::{iter, slice};

use strata_acct_types::{BitcoinAmount, Hash};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::EePredicateKeyUpdate;
use strata_codec::encode_to_vec;
use strata_da_framework::{DaCounter, DaLinacc, DaRegister};
use strata_identifiers::{AccountSerial, L1BlockCommitment, OLBlockCommitment, SubjectId};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockV1};
use strata_ol_da_common::{U16LenBytes, U16LenList};
use strata_ol_da_types_v1::{
    AccountDiffEntryV1, AccountDiffV1, DaProofStateDiffV1, GlobalStateDiffV1, LedgerDiffV1,
    OLDaPayloadV1, OLStateDiffV1, SnarkAccountDiffV1, decode_ol_da_payload_bytes,
};
use strata_ol_params::{OLParams, OLRuntimeParams};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IAccountState, ISnarkAccountState, IStateAccessor, OLSpecVersions};
use strata_ol_state_types_v1::{OLSnarkAccountStateV1, OLStateV1};
use strata_ol_stf_v1::process_block_manifests;
use strata_ol_stf_v1::test_utils::*;
use strata_ol_tx_types_v1::{
    OLTransactionDataV1, OLTransactionV1, TransactionPayloadV1, TxProofsV1,
};
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{
    BasicExecContext, BlockComponents, BlockContext, BlockInfo, CompletedBlock, EpochDaReplayError,
    EpochExecExpectations, EpochInfo, EpochL1Range, EpochSpecSelectionError, ExecError,
    ExecOutputBuffer, ExecResult, OLSpecId, TxExecContext, apply_da_epoch, construct_block,
    execute_and_complete_block, execute_block_batch_predrain, next_epoch_spec,
    select_next_epoch_spec, sequencer, verify_block, verify_epoch_with_diff,
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

/// Returns the commitment of the L1 block `manifest` is for.
fn manifest_block(manifest: &AsmManifest) -> L1BlockCommitment {
    L1BlockCommitment::new(manifest.height(), *manifest.blkid())
}

/// Selects the spec after a parent epoch whose terminal state runs `spec` and
/// which processed the L1 blocks after `prev_last` up to `last`, the block of
/// `manifest`.
fn select_after(
    spec: OLSpecId,
    prev_last: L1BlockCommitment,
    manifest: &AsmManifest,
) -> Result<OLSpecId, EpochSpecSelectionError<Infallible>> {
    let range = EpochL1Range::new(prev_last, manifest_block(manifest)).expect("ordered range");
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

    // This binary implements no spec after V1.
    let err = next_epoch_spec(OLSpecId::V1, Some(10)).expect_err("V1 has no known successor");
    assert_eq!(err.prev_spec(), OLSpecId::V1);
    assert_eq!(err.enactment_l1_height(), 10);
    assert_eq!(err.spec_version(), 2);
}

#[test]
fn test_select_switches_at_an_enactment_the_parent_processed() {
    let prev_last = manifest_block(&make_empty_manifest(9, 0));
    let enactment = make_checkpoint_predicate_enactment_manifest(10, 1);

    assert_eq!(
        select_after(OLSpecId::V0, prev_last, &enactment).expect("V0 has a successor"),
        OLSpecId::V1
    );

    // V1 -> V2: the epoch after a V1 epoch that processed an enactment runs
    // V2, which this binary does not implement.
    let err = select_after(OLSpecId::V1, prev_last, &enactment).expect_err("upgrade required");
    let EpochSpecSelectionError::UpgradeRequired(upgrade) = err else {
        panic!("expected an upgrade-required error, got {err}");
    };
    assert_eq!(upgrade.prev_spec(), OLSpecId::V1);
    assert_eq!(upgrade.enactment_l1_height(), 10);
    assert_eq!(upgrade.spec_version(), 2);
}

#[test]
fn test_select_keeps_the_spec_after_a_plain_last_manifest() {
    let prev_last = manifest_block(&make_empty_manifest(9, 0));
    let plain = make_empty_manifest(12, 1);
    for spec in [OLSpecId::V0, OLSpecId::V1] {
        assert_eq!(
            select_after(spec, prev_last, &plain).expect("selects"),
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
    for spec in [OLSpecId::V0, OLSpecId::V1] {
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
    let prev_last = manifest_block(&make_empty_manifest(9, 0));
    let plain = make_empty_manifest(10, 1);
    let range = EpochL1Range::new(prev_last, manifest_block(&plain)).expect("ordered range");
    let versions = OLSpecVersions::new(OLSpecId::V1, 7).expect("V1 may stage any version");
    let selected = select_next_epoch_spec(versions, range, |_| {
        Ok::<_, Infallible>(Some(plain.clone()))
    })
    .expect("selects");
    assert_eq!(selected, OLSpecId::V1);
}

#[test]
fn test_select_rejects_a_missing_or_foreign_last_manifest() {
    let prev_last = manifest_block(&make_empty_manifest(9, 0));
    let last = manifest_block(&make_empty_manifest(10, 1));
    let range = EpochL1Range::new(prev_last, last).expect("ordered range");
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
    let prev_last = manifest_block(&make_empty_manifest(9, 0));
    let duplicate = make_checkpoint_predicate_enactment_manifest(10, 2);
    let err = select_after(OLSpecId::V0, prev_last, &duplicate).expect_err("duplicate");
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
