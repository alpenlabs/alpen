//! Tests that pin mid-block failure semantics (no rollback + TxExec context wrapping).

use strata_acct_types::{AccountId, BitcoinAmount, TxEffects};
use strata_bridge_params::BridgeParams;
use strata_identifiers::Buf32;
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types::{
    BlockFlags, GamTxPayload, OLBlockBody, OLBlockHeader, OLTransaction, OLTransactionData,
    OLTxSegment, TransactionPayload, TxProofs,
};
use strata_ol_state_support_types::MemoryStateBaseLayer;

use crate::{
    assembly::BlockComponents, context::BlockInfo, errors::ExecError, test_utils::*, verify_block,
};

fn assert_mid_block_failure_state(
    state: &MemoryStateBaseLayer,
    snark_acct_id: AccountId,
    recipient_ok: AccountId,
    recipient_not_executed: AccountId,
) {
    // Intentional contract pin: STF mutates state in-place and caller (chain-worker)
    // owns any snapshot/rollback behavior around block processing.
    let (ledger_account_state, account_state) = state.expect_snark_account_state(snark_acct_id);
    assert_eq!(
        *account_state.seqno().inner(),
        1,
        "first tx should have advanced seqno"
    );
    assert_eq!(
        ledger_account_state.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "first tx should have deducted sender balance"
    );

    let recipient_ok_state = state
        .get_account_state(recipient_ok)
        .expect("recipient_ok lookup should succeed")
        .expect("recipient_ok should exist");
    assert_eq!(
        recipient_ok_state.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "first tx recipient should receive funds"
    );

    let recipient_not_executed_state = state
        .get_account_state(recipient_not_executed)
        .expect("recipient_not_executed lookup should succeed")
        .expect("recipient_not_executed should exist");
    assert_eq!(
        recipient_not_executed_state.balance(),
        BitcoinAmount::from_sat(0),
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
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_ok = make_account_id(TEST_RECIPIENT_ID);
    let recipient_not_executed = make_account_id(TEST_RECIPIENT_ID + 1);

    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_ok)
        .with_genesis_empty_account(recipient_not_executed)
        .execute_genesis();
    let mut state = fixture.state().clone();
    let genesis_block = fixture.last_completed_block().clone();

    let (_, account_state) = state.expect_snark_account_state(snark_acct_id);
    let account_state = account_state.clone();

    // tx0 succeeds, tx1 fails structural GAM checks, tx2 must never execute.
    let tx0 = SnarkUpdateBuilder::from_snark_state(account_state.clone())
        .with_transfer(recipient_ok, 10_000_000)
        .build(snark_acct_id, make_state_root(2), make_proof(1));
    let tx1 = make_invalid_gam_with_transfer(recipient_ok, recipient_ok);
    let tx1_id = tx1.compute_txid();
    let tx2 = SnarkUpdateBuilder::from_snark_state(account_state)
        .with_transfer(recipient_not_executed, 5_000_000)
        .build(snark_acct_id, make_state_root(4), make_proof(3));

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

    assert_mid_block_failure_state(&state, snark_acct_id, recipient_ok, recipient_not_executed);
}

#[test]
fn test_verify_block_mid_failure_returns_txexec() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_ok = make_account_id(TEST_RECIPIENT_ID);
    let recipient_not_executed = make_account_id(TEST_RECIPIENT_ID + 1);

    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_ok)
        .with_genesis_empty_account(recipient_not_executed)
        .execute_genesis();
    let mut state = fixture.state().clone();
    let genesis_block = fixture.last_completed_block().clone();

    // Build a terminal parent block so the next header advances again and passes
    // verify_header_continuity checks.
    let parent_info = BlockInfo::new(1_000_500, 1, 1);
    let parent_components = build_terminal_block_components(state.last_l1_height() + 1);
    let parent_block = execute_block(
        &mut state,
        &parent_info,
        Some(genesis_block.header()),
        parent_components,
    )
    .expect("terminal parent block should execute");

    let (_, account_state) = state.expect_snark_account_state(snark_acct_id);
    let account_state = account_state.clone();

    let tx0 = SnarkUpdateBuilder::from_snark_state(account_state.clone())
        .with_transfer(recipient_ok, 10_000_000)
        .build(snark_acct_id, make_state_root(2), make_proof(1));
    let tx1 = make_invalid_gam_with_transfer(recipient_ok, recipient_ok);
    let tx1_id = tx1.compute_txid();
    let tx2 = SnarkUpdateBuilder::from_snark_state(account_state)
        .with_transfer(recipient_not_executed, 5_000_000)
        .build(snark_acct_id, make_state_root(4), make_proof(3));

    let body = OLBlockBody::new_common(
        OLTxSegment::new(vec![tx0, tx1, tx2]).expect("tx segment should be within limits"),
    );
    let header = OLBlockHeader::new(
        1_001_000,
        BlockFlags::zero(),
        parent_block.header().slot() + 1,
        parent_block.header().epoch() + u32::from(parent_block.header().is_terminal()),
        parent_block.header().compute_blkid(),
        body.compute_hash_commitment(),
        Buf32::zero(),
        Buf32::zero(),
    );

    let result = verify_block(
        &mut state,
        &header,
        Some(parent_block.header()),
        &body,
        BridgeParams::default(),
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
        Err(err) => panic!("Expected TxExec wrapper from verify_block, got: {err:?}"),
        Ok(_) => panic!("verify_block should fail on second tx"),
    }

    assert_mid_block_failure_state(&state, snark_acct_id, recipient_ok, recipient_not_executed);
}
