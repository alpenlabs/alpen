//! Applying a DA blob built via the checkpoint-builder path to the pre-epoch
//! state, then replaying the epoch's manifests (buffer + epoch-terminal drain),
//! must reproduce the final epoch state root recorded in the terminal block.

use strata_acct_types::BitcoinAmount;
use strata_asm_common::AsmManifest;
use strata_bridge_params::BridgeParams;
use strata_codec::decode_buf_exact;
use strata_identifiers::{Buf64, OLBlockCommitment, SubjectId};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{
    OLAsmManifestContainer, OLBlock, OLBlockHeader, SignedOLBlockHeader,
};
use strata_ol_da::{OLDaPayloadV1, OLDaSchemeV1};
use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};

use crate::{
    BlockInfo, EpochInfo,
    assembly::{BlockComponents, CompletedBlock},
    execute_block_batch_preseal,
    test_utils::*,
    verification::{EpochExecExpectations, verify_epoch_with_diff},
};

const SLOTS_PER_EPOCH: u64 = 10;
const GENESIS_TIMESTAMP: u64 = 1_000_000;
const SLOT_TIMESTAMP_STEP: u64 = 1_000;

#[test]
fn test_preseal_round_trip_with_deposit_manifest() {
    let fixture_builder = OLStfFixture::builder();
    let snark_acct_serial = fixture_builder.next_account_serial();
    let fixture = fixture_builder
        .with_genesis_snark_account(make_account_id(TEST_SNARK_ACCOUNT_ID), |acct| {
            acct.with_state_root(make_state_root(1))
        })
        .with_genesis_manifest(make_empty_manifest(1, 0))
        .execute_genesis();
    let mut state = fixture.state().clone();
    let genesis = fixture.last_completed_block().clone();
    let pre_epoch_state = state.clone();

    let (mut epoch_blocks, last_pre_terminal_header) =
        build_non_terminal_blocks(&mut state, &genesis);
    let terminal_manifest = make_deposit_manifest_for_account(
        state.last_l1_height() + 1,
        1,
        snark_acct_serial,
        SubjectId::from([42u8; 32]),
        BitcoinAmount::from_sat(150_000_000),
    );
    let terminal = execute_terminal(&mut state, &last_pre_terminal_header, terminal_manifest);
    epoch_blocks.push(to_ol_block(&terminal));

    assert_epoch_root_round_trip(&pre_epoch_state, &genesis, &epoch_blocks, &terminal);
}

#[test]
fn test_preseal_round_trip_with_limbo_deposit_manifest() {
    let fixture = OLStfFixture::builder()
        .with_genesis_manifest(make_empty_manifest(1, 0))
        .execute_genesis();
    let mut state = fixture.state().clone();
    let genesis = fixture.last_completed_block().clone();
    let pre_epoch_state = state.clone();

    let (mut epoch_blocks, last_pre_terminal_header) =
        build_non_terminal_blocks(&mut state, &genesis);
    let terminal_manifest = make_deposit_manifest_with_destination_bytes(
        state.last_l1_height() + 1,
        1,
        Vec::new(),
        BitcoinAmount::from_sat(75_000_000),
    );
    let terminal = execute_terminal(&mut state, &last_pre_terminal_header, terminal_manifest);
    epoch_blocks.push(to_ol_block(&terminal));

    assert_epoch_root_round_trip(&pre_epoch_state, &genesis, &epoch_blocks, &terminal);
}

fn assert_epoch_root_round_trip(
    pre_epoch_state: &MemoryStateBaseLayer,
    genesis: &CompletedBlock,
    epoch_blocks: &[OLBlock],
    terminal: &CompletedBlock,
) {
    // The terminal header commits the single final epoch state root.
    let expected_root = *terminal.header().state_root();

    // The DA blob excludes the terminal drain effects; the epoch's manifests
    // are replayed (buffer + drain) on the verify side to reproduce them.
    let manifests = terminal
        .body()
        .manifests()
        .cloned()
        .unwrap_or_else(|| OLAsmManifestContainer::new(vec![]).expect("empty manifests"));

    let da_blob = rebuild_da_blob(pre_epoch_state, epoch_blocks, genesis.header());
    let payload: OLDaPayloadV1 = decode_buf_exact(&da_blob).expect("decode DA payload");

    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal.header()),
        OLBlockCommitment::new(genesis.header().slot(), genesis.header().compute_blkid()),
    );
    let mut verify_state = pre_epoch_state.clone();
    let exp = EpochExecExpectations::new(expected_root);
    let result = verify_epoch_with_diff::<_, OLDaSchemeV1>(
        &mut verify_state,
        &epoch_info,
        payload,
        &manifests,
        &exp,
    );

    let actual_root = verify_state
        .compute_state_root()
        .expect("verification state root should compute");
    result.unwrap_or_else(|e| {
        panic!("epoch root mismatch: {e:?}. expected = {expected_root:?}, actual = {actual_root:?}")
    });
}

fn build_non_terminal_blocks(
    state: &mut MemoryStateBaseLayer,
    genesis: &CompletedBlock,
) -> (Vec<OLBlock>, OLBlockHeader) {
    let mut prev_header = genesis.header().clone();
    let mut blocks = Vec::with_capacity(SLOTS_PER_EPOCH as usize);

    for slot in 1..SLOTS_PER_EPOCH {
        let cb = execute_block(
            state,
            &BlockInfo::new(GENESIS_TIMESTAMP + slot * SLOT_TIMESTAMP_STEP, slot, 1),
            Some(&prev_header),
            BlockComponents::new_empty(),
        )
        .expect("intra-epoch block");
        blocks.push(to_ol_block(&cb));
        prev_header = cb.header().clone();
    }

    (blocks, prev_header)
}

fn execute_terminal(
    state: &mut MemoryStateBaseLayer,
    parent_header: &OLBlockHeader,
    manifest: AsmManifest,
) -> CompletedBlock {
    execute_block(
        state,
        &BlockInfo::new(
            GENESIS_TIMESTAMP + SLOTS_PER_EPOCH * SLOT_TIMESTAMP_STEP,
            SLOTS_PER_EPOCH,
            1,
        ),
        Some(parent_header),
        BlockComponents::new_manifests(vec![manifest]).as_terminal(),
    )
    .expect("terminal block")
}

fn rebuild_da_blob(
    pre_epoch_state: &MemoryStateBaseLayer,
    blocks: &[OLBlock],
    prev_terminal_header: &OLBlockHeader,
) -> Vec<u8> {
    let mut da = DaAccumulatingState::new(pre_epoch_state.clone());
    execute_block_batch_preseal(
        &mut da,
        blocks,
        prev_terminal_header,
        BridgeParams::default(),
    )
    .expect("execute_block_batch_preseal");
    da.take_completed_epoch_da_blob()
        .expect("finalize DA")
        .expect("DA blob")
}

fn to_ol_block(cb: &CompletedBlock) -> OLBlock {
    OLBlock::new(
        SignedOLBlockHeader::new(cb.header().clone(), Buf64::zero()),
        cb.body().clone(),
    )
}
