//! Minimal deposit transaction builders for testing

use std::collections::HashMap;

use bitcoin::{Address, Amount, OutPoint, Transaction, hashes::Hash};
use strata_crypto::{EvenSecretKey, test_utils::schnorr::Musig2Tweak};
use strata_l1_txfmt::ParseConfig;
use strata_primitives::constants::RECOVER_DELAY;
use strata_test_utils_btcio::{BtcioTestHarness, address::derive_musig2_p2tr_address};

use crate::{
    deposit::DepositTxHeaderAux,
    deposit_request::{DRT_OUTPUT_INDEX, DrtHeaderAux, build_deposit_request_spend_info},
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx, create_test_deposit_request_tx},
};

/// Creates a deposit request transaction and its matching deposit transaction, wiring them
/// together.
///
/// Returns the tuple `(drt, dt)` so test cases can inspect both the funding request and the final
/// deposit submission.
pub fn create_connected_drt_and_dt(
    drt_header_aux: DrtHeaderAux,
    dt_header_aux: DepositTxHeaderAux,
    deposit_amount: Amount,
    operator_keys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    let harness =
        BtcioTestHarness::new_with_coinbase_maturity().expect("regtest harness should start");

    let (nn_address, internal_key) =
        derive_musig2_p2tr_address(operator_keys).expect("operator keys must aggregate");
    let mut drt = create_test_deposit_request_tx(&drt_header_aux, internal_key, deposit_amount);

    let drt_txid = harness
        .submit_transaction_with_keys_blocking(operator_keys, &mut drt, None)
        .expect("DRT submission should succeed");

    let mut dt = create_test_deposit_tx(dt_header_aux, nn_address, deposit_amount);

    let drt_inpoint = OutPoint {
        txid: drt_txid,
        vout: DRT_OUTPUT_INDEX as u32,
    };
    // Wire the deposit transaction to the confirmed DRT output so tests see a valid spend chain.
    dt.input[0].previous_output = drt_inpoint;

    // Keep the Musig2 tweaks we need to sign the deposit transaction after the DRT confirms.
    let mut input_tweaks = HashMap::new();
    let spend_info =
        build_deposit_request_spend_info(drt_header_aux.recovery_pk(), internal_key, RECOVER_DELAY);
    if let Some(root) = spend_info.merkle_root() {
        input_tweaks.insert(
            drt_inpoint,
            Musig2Tweak::TaprootScript(root.to_raw_hash().to_byte_array()),
        );
    }
    let _ = harness
        .submit_transaction_with_keys_blocking(operator_keys, &mut dt, Some(&input_tweaks))
        .expect("DT submission should succeed");

    (drt, dt)
}

/// Creates a minimal deposit transaction that commits to the SPS50 tag data and NN output.
fn create_test_deposit_tx(
    dt_header_aux: DepositTxHeaderAux,
    nn_address: Address,
    deposit_amount: Amount,
) -> Transaction {
    let mut tx = create_dummy_tx(1, 2);

    let tag = dt_header_aux
        .build_tag_data()
        .expect("deposit tag must serialize");
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag.as_ref())
        .expect("encoding SPS50 script must succeed");

    tx.output[0].script_pubkey = sps_50_script;
    tx.output[1].script_pubkey = nn_address.script_pubkey();
    tx.output[1].value = deposit_amount;

    tx
}
