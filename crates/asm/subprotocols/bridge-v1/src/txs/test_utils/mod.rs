mod deposit;
mod utils;
mod withdrawal_fulfillment;

pub(crate) const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

pub(crate) use deposit::create_test_deposit_tx;
pub(crate) use utils::{create_tagged_payload, mutate_op_return_output, parse_tx};
pub(crate) use withdrawal_fulfillment::create_test_withdrawal_fulfillment_tx;
