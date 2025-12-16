use bitcoin::{
    Address, Amount, Network, OutPoint, Transaction, Witness, XOnlyPublicKey,
    hashes::{Hash as _, sha256},
    secp256k1::Secp256k1,
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
};
use strata_crypto::EvenSecretKey;
use strata_l1_txfmt::ParseConfig;
use strata_primitives::constants::UNSPENDABLE_PUBLIC_KEY;
use strata_test_utils_btcio::{
    BtcioTestHarness, address::derive_musig2_p2tr_address, create_dummy_tx,
    signing::sign_musig2_scriptpath,
};

use crate::{
    test_utils::TEST_MAGIC_BYTES,
    unstake::{UnstakeInfo, UnstakeTxHeaderAux, stake_connector_script},
};

/// Creates a connected pair of stake and unstake transactions for testing.
///
/// Returns a tuple `(stake_tx, unstake)` where `unstake` correctly spends
/// the stake output from `stake_tx`.
pub fn create_connected_stake_and_unstake_txs(
    header_aux: &UnstakeTxHeaderAux,
    operator_keys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    let harness =
        BtcioTestHarness::new_with_coinbase_maturity().expect("regtest harness should start");

    // Deterministic preimage ensures stake connector hash stays stable for tests.
    let preimage = [1u8; 32];
    let stake_hash = sha256::Hash::hash(&preimage).to_byte_array();
    let (_, nn_key) =
        derive_musig2_p2tr_address(operator_keys).expect("operator keys must aggregate");
    let (address, spend_info) = create_stake_connector_taproot_addr(stake_hash, nn_key);

    // 1. Create a stake transaction.
    let mut stake_tx = create_dummy_tx(0, 1);
    stake_tx.output[0].script_pubkey = address.script_pubkey();
    stake_tx.output[0].value = Amount::from_sat(1_000);

    let stake_txid = harness
        .submit_transaction_with_keys_blocking(operator_keys, &mut stake_tx, None)
        .expect("stake transaction submission should succeed");

    // 2. Create the base unstake transaction using the provided metadata.
    let unstake_info = UnstakeInfo::new(header_aux.clone(), nn_key);
    let mut unstake_tx = create_dummy_unstake_tx(&unstake_info);

    unstake_tx.input[0].previous_output = OutPoint {
        txid: stake_txid,
        vout: 0,
    };

    // Compute the script and sign with the correct leaf hash.
    let script = stake_connector_script(stake_hash, nn_key);

    let nn_sig = sign_musig2_scriptpath(
        &unstake_tx,
        operator_keys,
        &stake_tx.output,
        0,
        &script,
        LeafVersion::TapScript,
    )
    .expect("script path signature for unstake must succeed");

    // 4. Set the witness for the script path spend (script_sig is empty for taproot).
    // Use the same spend_info that was used to create the address.
    let control_block = spend_info
        .control_block(&(script.clone(), LeafVersion::TapScript))
        .expect("control block must exist for unstake script leaf");

    let mut witness_stack = Witness::new();
    witness_stack.push(preimage);
    witness_stack.push(nn_sig.serialize());
    witness_stack.push(script.to_bytes());
    witness_stack.push(control_block.serialize());

    unstake_tx.input[0].witness = witness_stack;

    // Broadcast the fully-signed transaction; no additional funding input is needed because the
    // stake output covers the fee.
    let _ = harness
        .broadcast_transaction(&unstake_tx)
        .expect("unstake transaction broadcast should succeed");

    (stake_tx, unstake_tx)
}

/// Creates an unstake transaction for testing purposes.
fn create_dummy_unstake_tx(info: &UnstakeInfo) -> Transaction {
    // Create a dummy tx with two inputs (placeholder at index 0, stake connector at index 1) and a
    // single output.
    let mut tx = create_dummy_tx(1, 1);

    // Encode auxiliary data and construct SPS 50 op_return script.
    let tag_data = info
        .header_aux()
        .build_tag_data()
        .expect("unstake header aux serialization must succeed");
    let op_return_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .expect("encoding SPS50 script must succeed");

    // The first output is SPS 50 header.
    tx.output[0].script_pubkey = op_return_script;

    tx
}

/// Creates the taproot spend info used by the stake connector script in tests.
///
/// Returns the corresponding address so callers can fund the script and the [`TaprootSpendInfo`]
/// required to later construct the control block for the script-path spend.
fn create_stake_connector_taproot_addr(
    stake_hash: [u8; 32],
    nn_pubkey: XOnlyPublicKey,
) -> (Address, TaprootSpendInfo) {
    let script = stake_connector_script(stake_hash, nn_pubkey);
    let secp = Secp256k1::new();
    let spend_info = TaprootBuilder::new()
        .add_leaf(0, script.clone())
        .expect("taproot builder should accept single leaf")
        .finalize(&secp, *UNSPENDABLE_PUBLIC_KEY)
        .expect("taproot spend info must finalize with unspendable key");

    let merkle_root = spend_info.merkle_root();

    let address = Address::p2tr(
        &secp,
        *UNSPENDABLE_PUBLIC_KEY,
        merkle_root,
        Network::Regtest,
    );

    (address, spend_info)
}
