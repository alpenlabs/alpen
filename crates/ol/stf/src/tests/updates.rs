//! Tests for successful update operations

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_state_types::OLState;

use crate::test_utils::{
    SnarkUpdateBuilder, create_empty_account, create_test_genesis_state, execute_tx_in_block,
    get_test_proof, get_test_recipient_account_id, get_test_snark_account_id, get_test_state_root,
    setup_genesis_with_snark_account,
};

#[test]
fn test_snark_update_success_with_transfer() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create valid update with transfer
    let transfer_amount = 30_000_000u64;
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, transfer_amount)
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
    assert!(
        result.is_ok(),
        "Valid update should succeed: {:?}",
        result.err()
    );

    // Verify balances
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(70_000_000),
        "Sender account balance should be 100M - 30M"
    );
    // Check the seq no of the sender
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient_account.balance(),
        BitcoinAmount::from_sat(30_000_000),
        "Recipient should receive 30M"
    );
}
