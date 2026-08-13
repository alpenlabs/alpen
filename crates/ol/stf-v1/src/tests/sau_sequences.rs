//! Multi-block sequence tests for dependent snark account updates.

use strata_acct_types::{BitcoinAmount, Hash};
use strata_bridge_params::BridgeParams;
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_ol_state_types::ISnarkAccountState;

use crate::{test_utils::*, verify_block};

fn inner_state_root_from_header(header: &OLBlockHeaderV1) -> Hash {
    Hash::from(header.state_root().0)
}

#[test]
fn test_dependent_snark_updates_advance_across_blocks() {
    const UPDATE_COUNT: u64 = 5;
    const INITIAL_BALANCE: u64 = 100_000_000;
    const TRANSFER_AMOUNT: u64 = 1_000_000;

    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(INITIAL_BALANCE)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();
    let mut verify_state = fixture.state().clone();
    let mut parent_header = fixture.parent_header().clone();

    for update_idx in 0..UPDATE_COUNT {
        let new_inner_state_root = inner_state_root_from_header(&parent_header);

        let slot = update_idx + 1;
        let block = fixture
            .child_block()
            .with_slot(slot)
            .with_epoch(1)
            .with_sau(snark_acct_id, |sau| {
                sau.transfer(
                    recipient_id,
                    BitcoinAmount::try_from(TRANSFER_AMOUNT)
                        .expect("amount must not exceed the Bitcoin money supply"),
                )
                .with_state_root(new_inner_state_root)
            })
            .execute()
            .completed_block()
            .clone();

        verify_block(
            &mut verify_state,
            block.header(),
            Some(&parent_header),
            block.body(),
            BridgeParams::default(),
        )
        .expect("dependent snark update block should verify");

        let account_state = fixture.expect_snark_account(snark_acct_id);
        assert_eq!(account_state.inner_state_root(), new_inner_state_root);

        parent_header = block.into_header();
    }

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::try_from(INITIAL_BALANCE - (UPDATE_COUNT * TRANSFER_AMOUNT))
            .expect("amount must not exceed the Bitcoin money supply")
    );
    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(*account_state.seqno().inner(), UPDATE_COUNT);
    assert_eq!(account_state.next_inbox_msg_idx(), 0);

    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::try_from(UPDATE_COUNT * TRANSFER_AMOUNT)
            .expect("amount must not exceed the Bitcoin money supply")
    );
}
