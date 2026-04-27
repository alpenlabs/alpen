//! Tests that pin mid-block failure semantics (no rollback + TxExec context wrapping).

use strata_acct_types::{AccountId, BitcoinAmount, TxEffects};
use strata_asm_common::AsmManifest;
use strata_identifiers::Buf32;
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::{
    BlockFlags, GamTxPayload, OLBlockBody, OLBlockHeader, OLTransaction, OLTransactionData,
    OLTxSegment, TransactionPayload, TxProofs,
};
use strata_ol_state_types::OLState;

use crate::{
    assembly::BlockComponents, context::BlockInfo, errors::ExecError, test_utils::*, verify_block,
};

fn assert_mid_block_failure_state(
    state: &mut OLState,
    snark_id: AccountId,
    recipient_ok: AccountId,
    recipient_not_executed: AccountId,
) {
    // Intentional contract pin: STF mutates state in-place and caller (chain-worker-new)
    // owns any snapshot/rollback behavior around block processing.
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(state, snark_id);
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "first tx should have advanced seqno"
    );
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "first tx should have deducted sender balance"
    );

    let recipient_ok_state = state.get_account_state(recipient_ok).unwrap().unwrap();
    assert_eq!(
        recipient_ok_state.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "first tx recipient should receive funds"
    );

    let recipient_not_executed_state = state
        .get_account_state(recipient_not_executed)
        .unwrap()
        .unwrap();
    assert_eq!(
        recipient_not_executed_state.balance(),
        BitcoinAmount::zero(),
        "third tx should not execute after mid-block failure"
    );
}

fn make_invalid_gam_with_transfer(target: AccountId, transfer_dest: AccountId) -> OLTransaction {
    let mut effects = TxEffects::default();
    effects.push_transfer(transfer_dest, 1);

    OLTransaction::new(
        OLTransactionData::new(
            TransactionPayload::GenericAccountMessage(
                GamTxPayload::new(target).expect("test GAM target should be valid"),
            ),
            effects,
        ),
        TxProofs::new_empty(),
    )
}

#[test]
fn test_execute_block_mid_failure_keeps_prior_mutations() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_ok = get_test_recipient_account_id();
    let recipient_not_executed = test_account_id(TEST_RECIPIENT_ID + 1);

    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    create_empty_account(&mut state, recipient_ok);
    create_empty_account(&mut state, recipient_not_executed);

    let snark_account_state = lookup_snark_state(&state, snark_id);

    // tx0 succeeds, tx1 fails structural GAM checks, tx2 must never execute.
    let tx0 = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_ok, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));
    let tx1 = make_invalid_gam_with_transfer(recipient_ok, recipient_ok);
    let tx1_id = tx1.compute_txid();
    let tx2 = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_not_executed, 5_000_000)
        .build(snark_id, get_test_state_root(4), get_test_proof(3));

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let result = execute_block(
        &mut state,
        &block_info,
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![tx0, tx1, tx2]),
    );

    match result {
        Err(ExecError::TxExec(txid, idx, inner)) => {
            assert_eq!(txid, tx1_id);
            assert_eq!(idx, 1, "failing tx index should be wrapped");
            match inner.as_ref() {
                ExecError::TxStructureCheckFailed(msg) => {
                    assert_eq!(*msg, "nonzero transfers");
                }
                err => panic!("Expected TxStructureCheckFailed inner error, got: {err:?}"),
            }
        }
        Err(err) => panic!("Expected TxExec wrapper, got: {err:?}"),
        Ok(_) => panic!("Block should fail on second tx"),
    }

    assert_mid_block_failure_state(&mut state, snark_id, recipient_ok, recipient_not_executed);
}

#[test]
fn test_verify_block_mid_failure_returns_txexec() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_ok = get_test_recipient_account_id();
    let recipient_not_executed = test_account_id(TEST_RECIPIENT_ID + 1);

    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    create_empty_account(&mut state, recipient_ok);
    create_empty_account(&mut state, recipient_not_executed);

    // Build a terminal parent block so the next header advances again and passes
    // verify_header_continuity checks.
    let parent_manifest = AsmManifest::new(
        1,
        test_l1_block_id(1),
        strata_identifiers::WtxidsRoot::from(Buf32::from([1u8; 32])),
        vec![],
    )
    .expect("test manifest should be valid");
    let parent_info = BlockInfo::new(1_000_500, 1, 1);
    let parent_block = execute_block(
        &mut state,
        &parent_info,
        Some(genesis_block.header()),
        BlockComponents::new_manifests(vec![parent_manifest]),
    )
    .expect("terminal parent block should execute");

    let snark_account_state = lookup_snark_state(&state, snark_id);

    let tx0 = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_ok, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));
    let tx1 = make_invalid_gam_with_transfer(recipient_ok, recipient_ok);
    let tx1_id = tx1.compute_txid();
    let tx2 = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_not_executed, 5_000_000)
        .build(snark_id, get_test_state_root(4), get_test_proof(3));

    let body = OLBlockBody::new_common(
        OLTxSegment::new(vec![tx0, tx1, tx2]).expect("tx segment should be within limits"),
    );
    let header = OLBlockHeader::new(
        1_001_000,
        BlockFlags::zero(),
        parent_block.header().slot() + 1,
        parent_block.header().epoch() + parent_block.header().is_terminal() as u32,
        parent_block.header().compute_blkid(),
        body.compute_hash_commitment(),
        Buf32::zero(),
        Buf32::zero(),
    );

    let result = verify_block(&mut state, &header, Some(parent_block.header()), &body);

    match result {
        Err(ExecError::TxExec(txid, idx, inner)) => {
            assert_eq!(txid, tx1_id);
            assert_eq!(idx, 1, "failing tx index should be wrapped");
            match inner.as_ref() {
                ExecError::TxStructureCheckFailed(msg) => {
                    assert_eq!(*msg, "nonzero transfers");
                }
                err => panic!("Expected TxStructureCheckFailed inner error, got: {err:?}"),
            }
        }
        Err(err) => panic!("Expected TxExec wrapper from verify_block, got: {err:?}"),
        Ok(_) => panic!("verify_block should fail on second tx"),
    }

    assert_mid_block_failure_state(&mut state, snark_id, recipient_ok, recipient_not_executed);
}
