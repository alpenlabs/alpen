mod deposit;
mod utils;
mod withdrawal_fulfillment;

pub const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

pub use deposit::create_test_deposit_tx;
pub use utils::{create_tagged_payload, mutate_op_return_output, parse_tx};
pub use withdrawal_fulfillment::{
    create_test_cooperative_tx, create_test_withdrawal_fulfillment_tx,
};
