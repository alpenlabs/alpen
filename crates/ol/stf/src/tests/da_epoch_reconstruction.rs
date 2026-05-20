//! Reconstructing an epoch from its DA diff via [`apply_da_epoch`] must yield
//! the same post-state root as direct block-by-block execution.
//!
//! Covers a deposit manifest only, a snark account update only, and both
//! combined. Each test builds a multi-block epoch with empty filler blocks
//! around the meaningful ones.

use strata_acct_types::{BitcoinAmount, MessageEntry, MsgPayload};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::DepositLog;
use strata_codec::decode_buf_exact;
use strata_identifiers::{
    AccountSerial, Buf32, OLBlockCommitment, SubjectId, SubjectIdBytes, WtxidsRoot,
};
use strata_ledger_types::{IStateAccessor, IStateAccessorMut, NewAccountData, NewAccountTypeState};
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_chain_types_new::{
    L1BlockId, OLBlock, OLBlockHeader, OLTransaction, OLTransactionData, TxProofs,
};
use strata_ol_da::{OLDaPayloadV1, OLDaSchemeV1};
use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};
use strata_predicate::PredicateKey;

use crate::{
    BlockInfo, EpochInfo, SEQUENCER_ACCT_ID, apply_da_epoch,
    assembly::{BlockComponents, CompletedBlock},
    execute_block_batch_preseal,
    test_utils::{
        InboxMmrTracker, SnarkUpdateBuilder, TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID,
        execute_block, get_snark_state_expect, insert_empty_account, make_account_id,
        make_empty_manifest, make_genesis_state, make_state_root, to_ol_block,
    },
};

const GENESIS_TIMESTAMP: u64 = 1_000_000;
const SLOT_TIMESTAMP_STEP: u64 = 1_000;

/// L1 height of an epoch's terminal manifest in these tests.
///
/// Genesis carries the manifest at height 1; the non-terminal blocks of the
/// epoch under test carry none, so the terminal manifest is always at height 2.
const TERMINAL_L1_HEIGHT: u32 = 2;

#[test]
fn test_apply_da_epoch_deposit_manifest_only() {
    let mut state = make_genesis_state();
    let snark_serial = seed_accounts(&mut state);
    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone();

    let mut blocks = Vec::new();
    let mut prev = genesis.header().clone();
    for _ in 0..4 {
        prev = run_block(&mut state, &mut blocks, &prev, BlockComponents::new_empty());
    }
    let terminal = run_terminal(
        &mut state,
        &mut blocks,
        &prev,
        deposit_manifest(TERMINAL_L1_HEIGHT, snark_serial),
    );

    assert_reconstruction_matches(&state, &pre_epoch_state, &genesis, &terminal, &blocks);
}

#[test]
fn test_apply_da_epoch_snark_update_only() {
    let mut state = make_genesis_state();
    seed_accounts(&mut state);
    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone();

    let mut blocks = Vec::new();
    let prev = run_snark_update_blocks(&mut state, &mut blocks, genesis.header());
    let terminal = run_terminal(
        &mut state,
        &mut blocks,
        &prev,
        make_empty_manifest(TERMINAL_L1_HEIGHT, 0),
    );

    assert_reconstruction_matches(&state, &pre_epoch_state, &genesis, &terminal, &blocks);
}

#[test]
fn test_apply_da_epoch_snark_update_and_deposit() {
    let mut state = make_genesis_state();
    let snark_serial = seed_accounts(&mut state);
    let genesis = run_genesis(&mut state);
    let pre_epoch_state = state.clone();

    let mut blocks = Vec::new();
    let prev = run_snark_update_blocks(&mut state, &mut blocks, genesis.header());
    let terminal = run_terminal(
        &mut state,
        &mut blocks,
        &prev,
        deposit_manifest(TERMINAL_L1_HEIGHT, snark_serial),
    );

    assert_reconstruction_matches(&state, &pre_epoch_state, &genesis, &terminal, &blocks);
}

/// Guards against a silent no-op: if the snark update or deposit produced no
/// state change, the tests above would still pass. Distinct roots prove each
/// path genuinely mutates state.
#[test]
fn test_apply_da_epoch_cases_produce_distinct_roots() {
    let deposit_only = {
        let mut state = make_genesis_state();
        let snark_serial = seed_accounts(&mut state);
        let genesis = run_genesis(&mut state);
        let pre_epoch_state = state.clone();
        let mut blocks = Vec::new();
        let mut prev = genesis.header().clone();
        for _ in 0..4 {
            prev = run_block(&mut state, &mut blocks, &prev, BlockComponents::new_empty());
        }
        let terminal = run_terminal(
            &mut state,
            &mut blocks,
            &prev,
            deposit_manifest(TERMINAL_L1_HEIGHT, snark_serial),
        );
        reconstruct_epoch(&pre_epoch_state, &genesis, &terminal, &blocks)
    };
    let snark_only = {
        let mut state = make_genesis_state();
        seed_accounts(&mut state);
        let genesis = run_genesis(&mut state);
        let pre_epoch_state = state.clone();
        let mut blocks = Vec::new();
        let prev = run_snark_update_blocks(&mut state, &mut blocks, genesis.header());
        let terminal = run_terminal(
            &mut state,
            &mut blocks,
            &prev,
            make_empty_manifest(TERMINAL_L1_HEIGHT, 0),
        );
        reconstruct_epoch(&pre_epoch_state, &genesis, &terminal, &blocks)
    };
    let snark_and_deposit = {
        let mut state = make_genesis_state();
        let snark_serial = seed_accounts(&mut state);
        let genesis = run_genesis(&mut state);
        let pre_epoch_state = state.clone();
        let mut blocks = Vec::new();
        let prev = run_snark_update_blocks(&mut state, &mut blocks, genesis.header());
        let terminal = run_terminal(
            &mut state,
            &mut blocks,
            &prev,
            deposit_manifest(TERMINAL_L1_HEIGHT, snark_serial),
        );
        reconstruct_epoch(&pre_epoch_state, &genesis, &terminal, &blocks)
    };

    assert_ne!(deposit_only, snark_only);
    assert_ne!(deposit_only, snark_and_deposit);
    assert_ne!(snark_only, snark_and_deposit);
}

/// Runs the non-terminal blocks of a snark-update epoch.
/// Returns the header of the last block, for the terminal to build on.
fn run_snark_update_blocks(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    genesis_header: &OLBlockHeader,
) -> OLBlockHeader {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let inbox_msg = snark_inbox_msg();

    let mut prev = run_block(state, blocks, genesis_header, BlockComponents::new_empty());

    let gam_tx = OLTransaction::new(
        OLTransactionData::from_gam_bytes(snark_id, inbox_msg.payload().data().to_vec())
            .expect("gam payload"),
        TxProofs::new_empty(),
    );
    prev = run_block(state, blocks, &prev, txs_components(gam_tx));

    prev = run_block(state, blocks, &prev, BlockComponents::new_empty());

    // The GAM ran above, so the snark account's live state now exists.
    let update_tx = build_snark_update(state, &inbox_msg);
    run_block(state, blocks, &prev, txs_components(update_tx))
}

/// Builds a snark account update tx consuming the single inbox message from
/// `state`'s live snark account.
fn build_snark_update(state: &MemoryStateBaseLayer, inbox_msg: &MessageEntry) -> OLTransaction {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    // A one-message MMR yields the proof for the message delivered by the GAM;
    // empty filler blocks do not touch the inbox, so it stays at index 0.
    let mut inbox_tracker = InboxMmrTracker::new();
    let proof = inbox_tracker.add_message(inbox_msg);

    let (_, snark_state) = get_snark_state_expect(state, snark_id);
    SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_processed_msgs(vec![inbox_msg.clone()])
        .with_inbox_proofs(vec![proof])
        .with_transfer(make_account_id(TEST_RECIPIENT_ID), 1_000_000)
        .build(snark_id, make_state_root(2), vec![0u8; 32])
}

/// Seeds the recipient and snark accounts, returning the snark account serial.
fn seed_accounts(state: &mut MemoryStateBaseLayer) -> AccountSerial {
    insert_empty_account(state, make_account_id(TEST_RECIPIENT_ID));
    state
        .create_new_account(
            make_account_id(TEST_SNARK_ACCOUNT_ID),
            NewAccountData::new(
                BitcoinAmount::from_sat(100_000_000),
                NewAccountTypeState::Snark {
                    update_vk: PredicateKey::always_accept(),
                    initial_state_root: make_state_root(1),
                },
            ),
        )
        .expect("create snark account")
}

/// Executes the genesis (epoch 0 terminal) block.
fn run_genesis(state: &mut MemoryStateBaseLayer) -> CompletedBlock {
    execute_block(
        state,
        &BlockInfo::new_genesis(GENESIS_TIMESTAMP),
        None,
        BlockComponents::new_manifests(vec![make_empty_manifest(1, 0)]),
    )
    .expect("genesis block")
}

/// Executes one block at the slot following `parent` with the given
/// fully-formed `components`, appends it to `blocks`, and returns its header.
fn run_block(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    parent: &OLBlockHeader,
    components: BlockComponents,
) -> OLBlockHeader {
    let slot = parent.slot() + 1;
    let cb = execute_block(
        state,
        &BlockInfo::new(GENESIS_TIMESTAMP + slot * SLOT_TIMESTAMP_STEP, slot, 1),
        Some(parent),
        components,
    )
    .expect("epoch block");
    blocks.push(to_ol_block(&cb));
    cb.header().clone()
}

/// Executes the terminal block carrying `manifest`, closing the epoch.
fn run_terminal(
    state: &mut MemoryStateBaseLayer,
    blocks: &mut Vec<OLBlock>,
    parent: &OLBlockHeader,
    manifest: AsmManifest,
) -> CompletedBlock {
    let slot = parent.slot() + 1;
    let cb = execute_block(
        state,
        &BlockInfo::new(GENESIS_TIMESTAMP + slot * SLOT_TIMESTAMP_STEP, slot, 1),
        Some(parent),
        BlockComponents::new_manifests(vec![manifest]),
    )
    .expect("terminal block");
    blocks.push(to_ol_block(&cb));
    cb
}

/// Reconstructs the epoch from its DA diff and returns the post-state root.
fn reconstruct_epoch(
    pre_epoch_state: &MemoryStateBaseLayer,
    genesis: &CompletedBlock,
    terminal: &CompletedBlock,
    blocks: &[OLBlock],
) -> Buf32 {
    let mut da = DaAccumulatingState::new(pre_epoch_state.clone());
    execute_block_batch_preseal(&mut da, blocks, genesis.header())
        .expect("execute_block_batch_preseal");
    let da_blob = da
        .take_completed_epoch_da_blob()
        .expect("finalize DA")
        .expect("DA blob");
    let payload: OLDaPayloadV1 = decode_buf_exact(&da_blob).expect("decode DA payload");

    let epoch_info = EpochInfo::new(
        BlockInfo::from_header(terminal.header()),
        OLBlockCommitment::new(genesis.header().slot(), genesis.header().compute_blkid()),
    );
    let manifests = terminal
        .body()
        .l1_update()
        .expect("terminal must have l1_update")
        .manifest_cont()
        .clone();

    let mut reconstructed = pre_epoch_state.clone();
    apply_da_epoch::<_, OLDaSchemeV1>(&mut reconstructed, &epoch_info, payload, &manifests)
        .expect("apply_da_epoch");
    reconstructed.compute_state_root().unwrap()
}

/// Asserts the DA-reconstructed root equals the directly-executed root.
fn assert_reconstruction_matches(
    state: &MemoryStateBaseLayer,
    pre_epoch_state: &MemoryStateBaseLayer,
    genesis: &CompletedBlock,
    terminal: &CompletedBlock,
    blocks: &[OLBlock],
) {
    let direct_root = state.compute_state_root().unwrap();
    assert_eq!(
        reconstruct_epoch(pre_epoch_state, genesis, terminal, blocks),
        direct_root,
        "DA-reconstructed state root must equal directly-executed root"
    );
}

/// The inbox message a GAM block delivers and the snark update consumes.
fn snark_inbox_msg() -> MessageEntry {
    MessageEntry::new(
        SEQUENCER_ACCT_ID,
        1,
        MsgPayload::from_bytes(BitcoinAmount::from_sat(0), b"inbox msg".to_vec())
            .expect("inbox msg payload"),
    )
}

/// Wraps a single transaction into block components.
fn txs_components(tx: OLTransaction) -> BlockComponents {
    BlockComponents::new_txs_from_ol_transactions(vec![tx])
}

fn deposit_manifest(height: u32, target_serial: AccountSerial) -> AsmManifest {
    let dest = SubjectIdBytes::try_new(SubjectId::from([42u8; 32]).inner().to_vec()).unwrap();
    let descriptor = DepositDescriptor::new(target_serial, dest).unwrap();
    let log_entry =
        AsmLogEntry::from_log(&DepositLog::new(descriptor.encode_to_varvec(), 150_000_000))
            .unwrap();
    AsmManifest::new(
        height,
        L1BlockId::from(Buf32::from([1u8; 32])),
        WtxidsRoot::from(Buf32::zero()),
        vec![log_entry],
    )
    .unwrap()
}
