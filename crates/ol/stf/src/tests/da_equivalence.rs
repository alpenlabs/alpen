//! Tests that DA reconstruction produces the same state as direct block execution.

use strata_identifiers::{Buf64, OLBlockCommitment};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{OLBlock, OLBlockBody, OLBlockHeader, SignedOLBlockHeader};
use strata_ol_da::{DaScheme, OLDaSchemeV1, decode_ol_da_payload_bytes};
use strata_ol_state_support_types::DaAccumulatingState;

use crate::assembly::{BlockComponents, CompletedBlock, execute_block_batch};
use crate::test_utils::{
    build_chain_with_transactions, create_test_genesis_state,
};
use crate::{
    BasicExecContext, BlockInfo, EpochInitialContext, ExecOutputBuffer, process_block_manifests,
    process_epoch_initial, verify_block,
};

/// Creates a genesis state with the same accounts that `build_chain_with_transactions` sets up.
fn create_state_matching_chain_setup() -> strata_ol_state_types::OLState {
    use strata_acct_types::BitcoinAmount;
    use strata_ledger_types::{AccountTypeState, NewAccountData};
    use strata_ol_state_types::OLSnarkAccountState;
    use strata_predicate::PredicateKey;

    use crate::test_utils::{
        create_empty_account, get_test_recipient_account_id, get_test_snark_account_id,
        get_test_state_root, test_account_id,
    };

    let mut state = create_test_genesis_state();

    // Mirror account creation from build_chain_with_transactions
    let snark_id = get_test_snark_account_id();
    let update_vk = PredicateKey::always_accept();
    let initial_state_root = get_test_state_root(1);
    let snark_state = OLSnarkAccountState::new_fresh(update_vk, initial_state_root);
    let balance = BitcoinAmount::from_sat(100_000_000);
    let new_acct_data = NewAccountData::new(balance, AccountTypeState::Snark(snark_state));
    state
        .create_new_account(snark_id, new_acct_data)
        .expect("create snark account");

    let gam_target = test_account_id(1);
    create_empty_account(&mut state, gam_target);
    let recipient_id = get_test_recipient_account_id();
    create_empty_account(&mut state, recipient_id);

    state
}

fn to_ol_block(header: &OLBlockHeader, body: &OLBlockBody) -> OLBlock {
    let signed = SignedOLBlockHeader::new(header.clone(), Buf64::zero());
    OLBlock::new(signed, body.clone())
}

/// Executes a single epoch's blocks via `DaAccumulatingState`, extracts the DA blob,
/// then reconstructs state from that blob using the 3-step DA pipeline.
/// Asserts both paths produce the same state root.
#[test]
fn da_reconstruction_matches_block_execution() {
    let slots_per_epoch = 4;
    // genesis (terminal, epoch 0) + 4 blocks (epoch 1, terminal at slot 4) = 5 blocks
    let num_blocks = 5;

    let mut exec_state = create_test_genesis_state();
    let blocks = build_chain_with_transactions(&mut exec_state, num_blocks, slots_per_epoch);

    // Split: genesis is epoch 0, blocks[1..5] are epoch 1.
    let genesis = &blocks[0];
    let epoch_blocks: Vec<&CompletedBlock> = blocks[1..].iter().collect();
    let terminal = epoch_blocks.last().unwrap();

    assert!(genesis.header().is_terminal(), "genesis must be terminal");
    assert!(terminal.header().is_terminal(), "last block must be terminal");

    // State root after full execution (reference).
    let exec_root = exec_state.compute_state_root().expect("exec state root");

    // -- Execute epoch 0 (genesis) on both DA-tracking and reconstruction states --
    let mut da_state = create_state_matching_chain_setup();
    let mut da_acc = DaAccumulatingState::new(da_state);
    let genesis_ol = to_ol_block(genesis.header(), genesis.body());
    verify_block(&mut da_acc, genesis_ol.header(), None, genesis_ol.body())
        .expect("genesis verify");

    let genesis_blob = da_acc
        .take_completed_epoch_da_blob()
        .expect("take genesis blob")
        .expect("genesis should produce a blob");
    da_state = da_acc.into_inner();

    // -- Execute epoch 1 blocks via DaAccumulatingState --
    let ol_blocks: Vec<OLBlock> = epoch_blocks
        .iter()
        .map(|b| to_ol_block(b.header(), b.body()))
        .collect();

    let mut da_acc = DaAccumulatingState::new(da_state);
    execute_block_batch(&mut da_acc, &ol_blocks, genesis.header()).expect("batch execution");

    let epoch1_blob = da_acc
        .take_completed_epoch_da_blob()
        .expect("take epoch1 blob")
        .expect("epoch 1 should produce a blob");
    da_state = da_acc.into_inner();

    // Sanity: DA-path execution matches direct execution.
    let da_exec_root = da_state.compute_state_root().expect("da exec state root");
    assert_eq!(exec_root, da_exec_root, "DA-tracked execution must match direct execution");

    // -- Reconstruct entirely from DA blobs --
    // Pre-genesis account setup is not captured by DA blobs, so reconstruction
    // must start from the same initial state.
    let mut recon_state = create_state_matching_chain_setup();

    // Epoch 0 reconstruction
    let payload0 = decode_ol_da_payload_bytes(&genesis_blob).expect("decode genesis blob");
    let ep0_commitment = OLBlockCommitment::new(0, genesis.header().compute_blkid());
    let epctx0 = EpochInitialContext::new(0, ep0_commitment);
    process_epoch_initial(&mut recon_state, &epctx0).expect("epoch 0 initial");
    OLDaSchemeV1::apply_to_state(payload0, &mut recon_state).expect("epoch 0 DA apply");
    let components0 = BlockComponents::from_block(&genesis_ol);
    if let Some(mf) = components0.manifest_container() {
        let outbuf = ExecOutputBuffer::new_empty();
        let blkinfo = BlockInfo::new(genesis.header().timestamp(), 0, 0);
        let exctx = BasicExecContext::new(blkinfo, &outbuf);
        process_block_manifests(&mut recon_state, mf, &exctx).expect("epoch 0 manifests");
    }

    // Epoch 1 reconstruction
    let payload1 = decode_ol_da_payload_bytes(&epoch1_blob).expect("decode epoch1 blob");
    let terminal_slot = terminal.header().slot();
    let ep1_commitment = OLBlockCommitment::new(terminal_slot, terminal.header().compute_blkid());
    let epctx1 = EpochInitialContext::new(1, ep1_commitment);
    process_epoch_initial(&mut recon_state, &epctx1).expect("epoch 1 initial");
    OLDaSchemeV1::apply_to_state(payload1, &mut recon_state).expect("epoch 1 DA apply");
    let terminal_ol = to_ol_block(terminal.header(), terminal.body());
    let components1 = BlockComponents::from_block(&terminal_ol);
    if let Some(mf) = components1.manifest_container() {
        let outbuf = ExecOutputBuffer::new_empty();
        let blkinfo = BlockInfo::new(terminal.header().timestamp(), terminal_slot, 1);
        let exctx = BasicExecContext::new(blkinfo, &outbuf);
        process_block_manifests(&mut recon_state, mf, &exctx).expect("epoch 1 manifests");
    }

    let recon_root = recon_state.compute_state_root().expect("recon state root");
    assert_eq!(exec_root, recon_root, "DA reconstruction must match direct execution");
}
