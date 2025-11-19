mod commit;
mod deposit;
mod utils;
mod withdrawal_fulfillment;

pub const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

pub use commit::create_test_commit_tx;
pub use deposit::create_test_deposit_tx;
pub use utils::{mutate_aux_data, parse_tx};
pub use withdrawal_fulfillment::create_test_withdrawal_fulfillment_tx;

// Core transaction builders (no DRT parsing, no signing)
pub use deposit::{DepositTxBuilderError, build_deposit_transaction, build_timelock_script};
pub use withdrawal_fulfillment::{
    WithdrawalInput, WithdrawalMetadata, WithdrawalTxBuilderError,
    create_simple_withdrawal_fulfillment_tx, create_withdrawal_fulfillment_tx,
};

// Utility functions
pub use utils::{create_tagged_payload, mutate_op_return_output, parse_tx};
