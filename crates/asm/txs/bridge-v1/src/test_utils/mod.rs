mod commit;
mod deposit;
mod utils;
mod withdrawal_fulfillment;

pub const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

pub use commit::{create_test_commit_tx, setup_test_commit_tx};
pub use deposit::create_test_deposit_tx;
pub use utils::{
    create_tagged_payload, create_tx_with_n_of_n_multisig_output, mutate_op_return_output, parse_tx,
};
pub use withdrawal_fulfillment::create_test_withdrawal_fulfillment_tx;
