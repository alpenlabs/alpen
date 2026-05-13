//! Applying a DA blob built via the checkpoint-builder path to the
//! pre-epoch state must reproduce the preseal root recorded in the terminal
//! block.

use strata_acct_types::BitcoinAmount;
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::DepositLog;
use strata_codec::{VarVec, decode_buf_exact};
use strata_identifiers::{
    AccountSerial, Buf32, Buf64, OLBlockCommitment, SubjectId, SubjectIdBytes, WtxidsRoot,
};
use strata_ledger_types::{IStateAccessor, IStateAccessorMut, NewAccountData, NewAccountTypeState};
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_chain_types_new::{L1BlockId, OLBlock, OLBlockHeader, SignedOLBlockHeader};
use strata_ol_da::{OLDaPayloadV1, OLDaSchemeV1};
use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};
use strata_predicate::PredicateKey;

use crate::{
    BlockInfo, EpochInfo,
    assembly::{BlockComponents, CompletedBlock},
    execute_block_batch_preseal,
    test_utils::{
        create_test_genesis_state, execute_block, get_test_snark_account_id, get_test_state_root,
        test_l1_block_id,
    },
    verification::verify_epoch_preseal_with_diff,
};

const SLOTS_PER_EPOCH: u64 = 10;
const GENESIS_TIMESTAMP: u64 = 1_000_000;
const SLOT_TIMESTAMP_STEP: u64 = 1_000;

#[test]
fn test_preseal_round_trip_with_deposit_manifest() {
    let mut state = create_test_genesis_state();
    let snark_serial = seed_snark_account(&mut state);

    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone();

    let (mut epoch_blocks, last_pre_terminal_header) =
        build_non_terminal_blocks(&mut state, &genesis);
    let terminal_manifest = deposit_to_account_manifest(state.last_l1_height() + 1, snark_serial);
    let terminal = execute_terminal(&mut state, &last_pre_terminal_header, terminal_manifest);
    epoch_blocks.push(to_ol_block(&terminal));

    assert_preseal_round_trip(&pre_epoch_state, &genesis, &epoch_blocks, &terminal);
}

#[test]
fn test_preseal_round_trip_with_limbo_deposit_manifest() {
    let mut state = create_test_genesis_state();

    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone();

    let (mut epoch_blocks, last_pre_terminal_header) =
        build_non_terminal_blocks(&mut state, &genesis);
    let terminal_manifest = malformed_deposit_manifest(state.last_l1_height() + 1);
    let terminal = execute_terminal(&mut state, &last_pre_terminal_header, terminal_manifest);
    epoch_blocks.push(to_ol_block(&terminal));

    assert_preseal_round_trip(&pre_epoch_state, &genesis, &epoch_blocks, &terminal);
}

fn assert_preseal_round_trip(
    pre_epoch_state: &MemoryStateBaseLayer,
    genesis: &CompletedBlock,
    epoch_blocks: &[OLBlock],
    terminal: &CompletedBlock,
) {
    let preseal_recorded = *terminal
        .body()
        .l1_update()
        .expect("terminal must have l1_update")
        .preseal_state_root();

    let da_blob = rebuild_da_blob(pre_epoch_state, epoch_blocks, genesis.header());
    let payload: OLDaPayloadV1 = decode_buf_exact(&da_blob).expect("decode DA payload");

    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal.header()),
        OLBlockCommitment::new(genesis.header().slot(), genesis.header().compute_blkid()),
    );
    let mut verify_state = pre_epoch_state.clone();
    let result = verify_epoch_preseal_with_diff::<_, OLDaSchemeV1>(
        &mut verify_state,
        &epoch_info,
        payload,
        &preseal_recorded,
    );

    let actual_root = verify_state.compute_state_root().unwrap();
    result.unwrap_or_else(|e| {
        panic!("preseal mismatch: {e:?}. recorded = {preseal_recorded:?}, actual = {actual_root:?}")
    });
}

fn seed_snark_account(state: &mut MemoryStateBaseLayer) -> AccountSerial {
    state
        .create_new_account(
            get_test_snark_account_id(),
            NewAccountData::new(
                BitcoinAmount::from_sat(0),
                NewAccountTypeState::Snark {
                    update_vk: PredicateKey::always_accept(),
                    initial_state_root: get_test_state_root(1),
                },
            ),
        )
        .expect("create snark account")
}

fn run_genesis(state: &mut MemoryStateBaseLayer) -> CompletedBlock {
    execute_block(
        state,
        &BlockInfo::new_genesis(GENESIS_TIMESTAMP),
        None,
        BlockComponents::new_manifests(vec![empty_manifest(1)]),
    )
    .expect("genesis block")
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
        BlockComponents::new_manifests(vec![manifest]),
    )
    .expect("terminal block")
}

fn rebuild_da_blob(
    pre_epoch_state: &MemoryStateBaseLayer,
    blocks: &[OLBlock],
    prev_terminal_header: &OLBlockHeader,
) -> Vec<u8> {
    let mut da = DaAccumulatingState::new(pre_epoch_state.clone());
    execute_block_batch_preseal(&mut da, blocks, prev_terminal_header)
        .expect("execute_block_batch_preseal");
    da.take_completed_epoch_da_blob()
        .expect("finalize DA")
        .expect("DA blob")
}

fn empty_manifest(height: u32) -> AsmManifest {
    AsmManifest::new(
        height,
        L1BlockId::from(Buf32::zero()),
        WtxidsRoot::from(Buf32::zero()),
        vec![],
    )
    .expect("manifest")
}

fn deposit_to_account_manifest(height: u32, target_serial: AccountSerial) -> AsmManifest {
    let dest = SubjectIdBytes::try_new(SubjectId::from([42u8; 32]).inner().to_vec()).unwrap();
    let descriptor = DepositDescriptor::new(target_serial, dest).unwrap();
    let log_entry =
        AsmLogEntry::from_log(&DepositLog::new(descriptor.encode_to_varvec(), 150_000_000))
            .unwrap();
    AsmManifest::new(
        height,
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::zero()),
        vec![log_entry],
    )
    .unwrap()
}

fn malformed_deposit_manifest(height: u32) -> AsmManifest {
    let bogus_destination = VarVec::from_vec(Vec::<u8>::new()).expect("varvec");
    let log_entry = AsmLogEntry::from_log(&DepositLog::new(bogus_destination, 75_000_000)).unwrap();
    AsmManifest::new(
        height,
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::zero()),
        vec![log_entry],
    )
    .unwrap()
}

fn to_ol_block(cb: &CompletedBlock) -> OLBlock {
    OLBlock::new(
        SignedOLBlockHeader::new(cb.header().clone(), Buf64::zero()),
        cb.body().clone(),
    )
}
