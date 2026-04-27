//! Multi-block sequence tests for dependent snark account updates.

use strata_acct_types::{BitcoinAmount, Hash};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::OLBlockHeader;

use crate::{context::BlockInfo, test_utils::*, verify_block};

fn inner_state_root_from_header(header: &OLBlockHeader) -> Hash {
    Hash::from(header.state_root().0)
}

#[test]
fn test_dependent_snark_updates_advance_across_blocks() {
    const UPDATE_COUNT: u64 = 5;
    const INITIAL_BALANCE: u64 = 100_000_000;
    const TRANSFER_AMOUNT: u64 = 1_000_000;

    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    create_snark_account_with_balance(&mut state, snark_id, INITIAL_BALANCE);
    create_empty_account(&mut state, recipient_id);

    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_block = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("terminal genesis should execute");

    let mut verify_state = state.clone();
    let mut parent_header = genesis_block.into_header();

    for update_idx in 0..UPDATE_COUNT {
        let snark_account_state = lookup_snark_state(&state, snark_id);
        let new_inner_state_root = inner_state_root_from_header(&parent_header);
        let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
            .with_transfer(recipient_id, TRANSFER_AMOUNT)
            .build(
                snark_id,
                new_inner_state_root,
                get_test_proof(update_idx as u8 + 1),
            );

        let slot = update_idx + 1;
        let block = execute_tx_in_block(&mut state, &parent_header, tx, slot, 1)
            .expect("dependent snark update should execute");

        verify_block(
            &mut verify_state,
            block.header(),
            Some(&parent_header),
            block.body(),
        )
        .expect("dependent snark update block should verify");

        let snark_account_state = lookup_snark_state(&state, snark_id);
        assert_eq!(snark_account_state.inner_state_root(), new_inner_state_root);

        parent_header = block.into_header();
    }

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(INITIAL_BALANCE - (UPDATE_COUNT * TRANSFER_AMOUNT))
    );
    assert_eq!(*snark_account_state.seqno().inner(), UPDATE_COUNT);
    assert_eq!(snark_account_state.next_inbox_msg_idx(), 0);

    let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient_account.balance(),
        BitcoinAmount::from_sat(UPDATE_COUNT * TRANSFER_AMOUNT)
    );
}
