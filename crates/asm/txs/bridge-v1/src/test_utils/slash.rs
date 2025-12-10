use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, constants::COINBASE_MATURITY};
use secp256k1::SECP256K1;
use strata_codec::encode_to_vec;
use strata_crypto::EvenSecretKey;
use strata_l1_txfmt::{ParseConfig, TagData};
use strata_test_utils_btcio::{
    address::derive_musig2_p2tr_address, get_bitcoind_and_client, mining::mine_blocks_blocking,
    submit::submit_transaction_with_keys_blocking,
};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, SLASH_TX_TYPE},
    slash::{SlashInfo, SlashTxHeaderAux},
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
};

/// Creates a slash transaction for testing purposes.
pub fn create_test_slash_tx(info: &SlashInfo) -> Transaction {
    // Create a dummy tx with two inputs (contest connector at index 0, stake connector at index 1)
    // and a single output.
    let mut tx = create_dummy_tx(2, 1);

    // Encode auxiliary data and construct SPS 50 op_return script
    let aux_data = encode_to_vec(info.header_aux()).unwrap();
    let tag_data = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, SLASH_TX_TYPE, aux_data).unwrap();
    let op_return_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .unwrap();

    // The first output is SPS 50 header
    tx.output[0].script_pubkey = op_return_script;

    // The second input (index 1) is the stake connector
    tx.input[1].previous_output = info.second_inpoint().0;

    tx
}

/// Sets up a connected pair of stake and slash transactions for testing.
///
/// Returns a tuple `(stake_tx, slash_tx)` where `slash_tx` correctly spends
/// the stake output from `stake_tx`.
pub fn create_connected_stake_and_slash_txs(
    header_aux: &SlashTxHeaderAux,
    operator_keys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    let (bitcoind, client) = get_bitcoind_and_client();
    let _ =
        mine_blocks_blocking(&bitcoind, &client, (COINBASE_MATURITY + 1) as usize, None).unwrap();

    // 1. Create a "stake transaction" to act as the funding source. This simulates the N-of-N
    //    multisig UTXO that the slash transaction spends.
    let mut stake_tx = create_dummy_tx(0, 1);
    let (_, internal_key) = derive_musig2_p2tr_address(operator_keys).unwrap();
    let nn_script = ScriptBuf::new_p2tr(SECP256K1, internal_key, None);
    stake_tx.output[0].script_pubkey = nn_script;
    stake_tx.output[0].value = Amount::from_sat(1_000);

    let stake_txid =
        submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut stake_tx)
            .unwrap();

    // 2. Create the base slash transaction using the provided metadata.
    let slash_info = SlashInfo::new(header_aux.clone(), OutPoint::new(stake_txid, 0).into());
    let mut slash_tx = create_test_slash_tx(&slash_info);

    let _ = submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut slash_tx)
        .unwrap();

    dbg!(&slash_tx);

    (stake_tx, slash_tx)
}
