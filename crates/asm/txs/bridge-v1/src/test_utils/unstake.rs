use bitcoin::{
    Address, Amount, Network, OutPoint, Transaction, Witness, XOnlyPublicKey,
    constants::COINBASE_MATURITY,
    hashes::{Hash as _, sha256},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
};
use secp256k1::SECP256K1;
use strata_codec::encode_to_vec;
use strata_crypto::EvenSecretKey;
use strata_l1_txfmt::{ParseConfig, TagData};
use strata_primitives::constants::UNSPENDABLE_PUBLIC_KEY;
use strata_test_utils_btcio::{
    address::derive_musig2_p2tr_address, get_bitcoind_and_client, mining::mine_blocks_blocking,
    signing::sign_musig2_scriptpath, submit::submit_transaction_with_keys_blocking,
};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, UNSTAKE_TX_TYPE},
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
    unstake::{UnstakeInfo, UnstakeTxHeaderAux, stake_connector_script},
};

/// Sets up a connected pair of stake and unstake transactions for testing.
///
/// Returns a tuple `(stake_tx, unstake)` where `unstake` correctly spends
/// the stake output from `stake_tx`.
pub fn build_connected_stake_and_unstake_txs(
    header_aux: &UnstakeTxHeaderAux,
    operator_keys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    let (bitcoind, client) = get_bitcoind_and_client();
    let _ =
        mine_blocks_blocking(&bitcoind, &client, (COINBASE_MATURITY + 1) as usize, None).unwrap();

    let preimage = [1u8; 32];
    let stake_hash = sha256::Hash::hash(&preimage).to_byte_array();
    let (_, nn_key) = derive_musig2_p2tr_address(operator_keys).unwrap();
    let (address, spend_info) = stake_connector_tapproot_addr(stake_hash, nn_key);

    // 1. Create a stake transaction
    let mut stake_tx = create_dummy_tx(0, 1);
    stake_tx.output[0].script_pubkey = address.script_pubkey();
    stake_tx.output[0].value = Amount::from_sat(1_000);

    let stake_txid =
        submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut stake_tx)
            .unwrap();

    // 2. Create the base unstake transaction using the provided metadata.
    let unstake_info = UnstakeInfo::new(header_aux.clone(), nn_key);
    let mut unstake_tx = build_dummy_unstake_tx(&unstake_info);

    unstake_tx.input[0].previous_output = OutPoint {
        txid: stake_txid,
        vout: 0,
    };

    // Compute the script and sign with the correct leaf hash
    let script = stake_connector_script(stake_hash, nn_key);

    let nn_sig = sign_musig2_scriptpath(
        &unstake_tx,
        operator_keys,
        &stake_tx.output,
        0,
        &script,
        LeafVersion::TapScript,
    )
    .unwrap();

    // 4. Set the witness for the script path spend (script_sig is empty for taproot)
    // Use the same spend_info that was used to create the address
    let control_block = spend_info
        .control_block(&(script.clone(), LeafVersion::TapScript))
        .unwrap();

    let mut witness_stack = Witness::new();
    witness_stack.push(preimage);
    witness_stack.push(nn_sig.serialize());
    witness_stack.push(script.to_bytes());
    witness_stack.push(control_block.serialize());

    unstake_tx.input[0].witness = witness_stack;

    // Broadcast the fully-signed transaction; no additional funding input is needed because the
    // stake output covers the fee.
    let _ = bitcoind.client.send_raw_transaction(&unstake_tx).unwrap();

    (stake_tx, unstake_tx)
}

/// Creates an unstake transaction for testing purposes.
fn build_dummy_unstake_tx(info: &UnstakeInfo) -> Transaction {
    // Create a dummy tx with two inputs (placeholder at index 0, stake connector at index 1) and a
    // single output.
    let mut tx = create_dummy_tx(1, 1);

    // Encode auxiliary data and construct SPS 50 op_return script
    let aux_data = encode_to_vec(info.header_aux()).unwrap();
    let tag_data = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, UNSTAKE_TX_TYPE, aux_data).unwrap();
    let op_return_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .unwrap();

    // The first output is SPS 50 header
    tx.output[0].script_pubkey = op_return_script;

    tx
}

fn stake_connector_tapproot_addr(
    stake_hash: [u8; 32],
    nn_pubkey: XOnlyPublicKey,
) -> (Address, TaprootSpendInfo) {
    let script = stake_connector_script(stake_hash, nn_pubkey);
    let spend_info = TaprootBuilder::new()
        .add_leaf(0, script.clone())
        .unwrap()
        .finalize(SECP256K1, *UNSPENDABLE_PUBLIC_KEY)
        .unwrap();

    let merkle_root = spend_info.merkle_root();

    let address = Address::p2tr(
        SECP256K1,
        *UNSPENDABLE_PUBLIC_KEY,
        merkle_root,
        Network::Regtest,
    );

    (address, spend_info)
}
