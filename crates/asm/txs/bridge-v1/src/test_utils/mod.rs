mod commit;
mod deposit;
mod slash;
mod unstake;
mod utils;
mod withdrawal_fulfillment;

pub const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

pub use commit::create_test_commit_tx;
pub use deposit::{build_deposit_transaction, create_deposit_op_return, create_test_deposit_tx};
pub use slash::{create_connected_stake_and_slash_txs, create_test_slash_tx};
pub use unstake::{create_connected_stake_and_unstake_txs, create_test_unstake_tx};
pub use utils::{create_dummy_tx, mutate_aux_data, parse_tx};
pub use withdrawal_fulfillment::create_test_withdrawal_fulfillment_tx;
// Withdrawal transaction builders
pub use withdrawal_fulfillment::{
    WithdrawalInput, WithdrawalMetadata, WithdrawalTxBuilderError,
    create_simple_withdrawal_fulfillment_tx, create_withdrawal_fulfillment_tx,
    create_withdrawal_op_return,
};
