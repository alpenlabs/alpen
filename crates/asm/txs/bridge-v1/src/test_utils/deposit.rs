//! Minimal deposit transaction builders for testing

use bitcoin::{Address, Amount, OutPoint, Transaction, constants::COINBASE_MATURITY};
use strata_crypto::EvenSecretKey;
use strata_l1_txfmt::ParseConfig;
use strata_test_utils_btcio::{
    address::derive_musig2_p2tr_address, get_bitcoind_and_client, mining::mine_blocks_blocking,
    submit::submit_transaction_with_keys_blocking,
};

use crate::{
    deposit::DepositTxHeaderAux,
    deposit_request::DrtHeaderAux,
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx, create_test_deposit_request_tx},
};

fn create_test_deposit_tx(
    dt_header_aux: DepositTxHeaderAux,
    nn_address: Address,
    deposit_amount: Amount,
) -> Transaction {
    let mut tx = create_dummy_tx(1, 2);

    let tag = dt_header_aux.build_tag_data().unwrap();
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag.as_ref())
        .unwrap();

    tx.output[0].script_pubkey = sps_50_script;
    tx.output[1].script_pubkey = nn_address.script_pubkey();
    tx.output[1].value = deposit_amount;

    tx
}

pub fn create_connected_drt_and_dt(
    drt_header_aux: DrtHeaderAux,
    dt_header_aux: DepositTxHeaderAux,
    deposit_amount: Amount,
    operator_keys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    let (bitcoind, client) = get_bitcoind_and_client();
    let _ =
        mine_blocks_blocking(&bitcoind, &client, (COINBASE_MATURITY + 1) as usize, None).unwrap();

    let (nn_address, internal_key) = derive_musig2_p2tr_address(operator_keys).unwrap();
    let mut drt = create_test_deposit_request_tx(&drt_header_aux, internal_key, deposit_amount);

    let drt_txid =
        submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut drt).unwrap();

    let mut dt = create_test_deposit_tx(dt_header_aux, nn_address, deposit_amount);
    dt.input[0].previous_output = OutPoint {
        txid: drt_txid,
        vout: 1,
    };

    let _ =
        submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut dt).unwrap();

    (drt, dt)
}
