//! Minimal deposit transaction builders for testing

use std::collections::HashMap;

use bitcoin::{Address, Amount, OutPoint, Transaction, constants::COINBASE_MATURITY, hashes::Hash};
use strata_crypto::{EvenSecretKey, test_utils::schnorr::Musig2Tweak};
use strata_l1_txfmt::ParseConfig;
use strata_primitives::constants::RECOVER_DELAY;
use strata_test_utils_btcio::{
    address::derive_musig2_p2tr_address, get_bitcoind_and_client, mining::mine_blocks_blocking,
    submit::submit_transaction_with_keys_blocking,
};

use crate::{
    deposit::DepositTxHeaderAux,
    deposit_request::{DRT_OUTPUT_INDEX, DrtHeaderAux, build_deposit_request_spend_info},
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
        submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut drt, None)
            .unwrap();

    let mut dt = create_test_deposit_tx(dt_header_aux, nn_address, deposit_amount);
    dt.input[0].previous_output = OutPoint {
        txid: drt_txid,
        vout: DRT_OUTPUT_INDEX as u32,
    };
    let mut input_tweaks = HashMap::new();
    let spend_info =
        build_deposit_request_spend_info(drt_header_aux.recovery_pk(), internal_key, RECOVER_DELAY);
    if let Some(root) = spend_info.merkle_root() {
        input_tweaks.insert(
            OutPoint {
                txid: drt_txid,
                vout: DRT_OUTPUT_INDEX as u32,
            },
            Musig2Tweak::TaprootScript(root.to_raw_hash().to_byte_array()),
        );
    }
    dbg!(&drt);
    dbg!(&drt.compute_txid());
    dbg!(&drt.compute_wtxid());

    dbg!(&dt);
    dbg!(&dt.compute_txid());
    dbg!(&dt.compute_wtxid());

    let _ = submit_transaction_with_keys_blocking(
        &bitcoind,
        &client,
        operator_keys,
        &mut dt,
        Some(&input_tweaks),
    )
    .unwrap();

    (drt, dt)
}
