mod deposit;
mod deposit_request;
mod slash;
mod unstake;
mod utils;
mod withdrawal_fulfillment;

pub const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

pub use deposit::create_connected_drt_and_dt;
pub use deposit_request::create_test_deposit_request_tx;
pub use slash::{create_connected_stake_and_slash_txs, create_test_slash_tx};
pub use strata_test_utils_btcio::create_dummy_tx;
pub use unstake::create_connected_stake_and_unstake_txs;
pub use utils::{create_test_operators, mutate_aux_data, parse_sps50_tx};
pub use withdrawal_fulfillment::create_test_withdrawal_fulfillment_tx;
