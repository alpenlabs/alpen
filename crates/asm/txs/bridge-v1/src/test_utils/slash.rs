use bitcoin::{Amount, OutPoint, Transaction};
use strata_asm_txs_test_utils::{TEST_MAGIC_BYTES, create_dummy_tx};
use strata_crypto::EvenSecretKey;
use strata_l1_txfmt::ParseConfig;
use strata_test_utils_btcio::{BtcioTestHarness, address::derive_musig2_p2tr_address};

use crate::slash::{SlashInfo, SlashTxHeaderAux};

/// Creates a slash transaction for testing purposes.
pub fn create_test_slash_tx(info: &SlashInfo) -> Transaction {
    // Create a dummy tx with two inputs (contest connector at index 0, stake connector at index 1)
    // and a single output.
    let mut tx = create_dummy_tx(2, 1);

    // Encode auxiliary data and construct SPS 50 op_return script.
    let tag_data = info.header_aux().build_tag_data();
    let op_return_script = ParseConfig::new(TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .expect("encoding SPS50 script must succeed");

    // The first output is SPS 50 header.
    tx.output[0].script_pubkey = op_return_script;

    // The second input (index 1) is the stake connector.
    tx.input[1].previous_output = info.stake_inpoint().0;

    tx
}

/// Creates a connected pair of stake and slash transactions for testing.
///
/// Returns a tuple `(stake_tx, slash_tx)` where `slash_tx` correctly spends
/// the stake output from `stake_tx`.
pub fn create_connected_stake_and_slash_txs(
    header_aux: &SlashTxHeaderAux,
    operator_keys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    let harness =
        BtcioTestHarness::new_with_coinbase_maturity().expect("regtest harness should start");

    // 1. Create a "stake transaction" to act as the funding source. This simulates the N-of-N
    //    multisig UTXO that the slash transaction spends.
    let mut stake_tx = create_dummy_tx(0, 1);
    let (address, _) =
        derive_musig2_p2tr_address(operator_keys).expect("operator keys must aggregate");
    stake_tx.output[0].script_pubkey = address.script_pubkey();
    stake_tx.output[0].value = Amount::from_sat(1_000);

    let stake_txid = harness
        .submit_transaction_with_keys_blocking(operator_keys, &mut stake_tx, None)
        .expect("stake transaction submission should succeed");

    // 2. Create the base slash transaction using the provided metadata.
    let slash_info = SlashInfo::new(header_aux.clone(), OutPoint::new(stake_txid, 0).into());
    let mut slash_tx = create_test_slash_tx(&slash_info);

    let _ = harness
        .submit_transaction_with_keys_blocking(operator_keys, &mut slash_tx, None)
        .expect("slash transaction submission should succeed");

    (stake_tx, slash_tx)
}
