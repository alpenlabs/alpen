use bitcoin::{
    Address, Amount, Network, OutPoint, ScriptBuf, Transaction, Witness, XOnlyPublicKey,
    constants::COINBASE_MATURITY,
    hashes::{Hash as _, sha256},
    opcodes::all::{OP_CHECKSIGVERIFY, OP_EQUALVERIFY, OP_PUSHNUM_1, OP_SHA256, OP_SIZE},
    secp256k1::schnorr::Signature,
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
    unstake::{UnstakeInfo, UnstakeTxHeaderAux},
};

/// Creates an unstake transaction for testing purposes.
pub fn create_test_unstake_tx(info: &UnstakeInfo) -> Transaction {
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

/// Sets up a connected pair of stake and slash transactions for testing.
///
/// Returns a tuple `(stake_tx, unstake)` where `unstake` correctly spends
/// the stake output from `stake_tx`.
pub fn create_connected_stake_and_unstake_txs(
    header_aux: &UnstakeTxHeaderAux,
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

    // let stake_txid =
    //     submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut stake_tx)
    //         .unwrap();

    // 2. Create the base slash transaction using the provided metadata.
    let unstake_info = UnstakeInfo::new(header_aux.clone());
    let mut unstake = create_test_unstake_tx(&unstake_info);

    let _ = submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut unstake)
        .unwrap();

    (stake_tx, unstake)
}

/// Sets up a connected pair of stake and unstake transactions for testing.
///
/// Returns a tuple `(stake_tx, unstake)` where `unstake` correctly spends
/// the stake output from `stake_tx`.
pub fn create_connected_stake_and_unstake_txs_new(
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
    let unstake_info = UnstakeInfo::new(header_aux.clone());
    let mut unstake_tx = create_test_unstake_tx(&unstake_info);

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

fn stake_connector_script(stake_hash: [u8; 32], nn_pubkey: XOnlyPublicKey) -> ScriptBuf {
    ScriptBuf::builder()
        // Verify the signature
        .push_slice(nn_pubkey.serialize())
        .push_opcode(OP_CHECKSIGVERIFY)
        // Verify size of preimage is 32 bytes
        .push_opcode(OP_SIZE)
        .push_int(0x20)
        .push_opcode(OP_EQUALVERIFY)
        // Verify the preimage matches the hash
        .push_opcode(OP_SHA256)
        .push_slice(stake_hash)
        .push_opcode(OP_EQUALVERIFY)
        // Leave a truthy stack element to satisfy cleanstack rules
        .push_opcode(OP_PUSHNUM_1)
        .into_script()
}

fn stake_connector_witness(
    preimage: [u8; 32],
    nn_pubkey: XOnlyPublicKey,
    nn_sig: Signature,
) -> Witness {
    let stake_hash = sha256::Hash::hash(&preimage).to_byte_array();
    let script = stake_connector_script(stake_hash, nn_pubkey);
    let (_, taproot_spending_info) = stake_connector_tapproot_addr(stake_hash, nn_pubkey);
    let control_block = taproot_spending_info
        .control_block(&(script.clone(), LeafVersion::TapScript))
        .unwrap();

    let mut witness_stack = Witness::new();
    witness_stack.push(preimage);
    witness_stack.push(nn_sig.serialize());
    witness_stack.push(script.to_bytes());
    witness_stack.push(control_block.serialize());

    witness_stack
}
