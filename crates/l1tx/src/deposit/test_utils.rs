use strata_primitives::{
    l1::{BitcoinAmount, BitcoinXOnlyPublicKey},
    params::DepositTxParams,
};
use strata_test_utils_btc::test_taproot_addr;

pub fn get_deposit_tx_config() -> DepositTxParams {
    DepositTxParams {
        magic_bytes: "ALPN".to_string().as_bytes().try_into().unwrap(),
        max_address_length: 20,
        deposit_amount: BitcoinAmount::from_sat(1_000_000_000),
        address: test_taproot_addr(),
        operators_pubkey: BitcoinXOnlyPublicKey::from_address(&test_taproot_addr()).unwrap(),
    }
}
