use bitcoin::{OutPoint, ScriptBuf, Transaction};
use strata_codec::encode_to_vec;
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, UNSTAKE_TX_TYPE},
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
    unstake::{UnstakeInfo, UnstakeTxHeaderAux},
};

/// Creates an unstake transaction for testing purposes.
pub fn create_test_unstake_tx(info: &UnstakeInfo) -> Transaction {
    // Create a dummy tx with two inputs (placeholder at index 0, stake connector at index 1) and a
    // single output.
    let mut tx = create_dummy_tx(2, 1);

    // Encode auxiliary data and construct SPS 50 op_return script
    let aux_data = encode_to_vec(info.header_aux()).unwrap();
    let tag_data = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, UNSTAKE_TX_TYPE, aux_data).unwrap();
    let op_return_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .unwrap();

    // The first output is SPS 50 header
    tx.output[0].script_pubkey = op_return_script;

    // The second input (index 1) is the stake connector
    tx.input[1].previous_output = info.second_inpoint().0;

    tx
}

/// Sets up a connected pair of stake and unstake transactions for testing.
///
/// Returns a tuple `(stake_tx, unstake_tx)` where `unstake_tx` correctly spends the stake output
/// from `stake_tx`.
pub fn create_connected_stake_and_unstake_txs(
    header_aux: &UnstakeTxHeaderAux,
    nn_script: ScriptBuf,
) -> (Transaction, Transaction) {
    // 1. Create a dummy "stake transaction" to act as the funding source. This simulates the N-of-N
    //    multisig UTXO that the unstake transaction spends. We explicitly set the script_pubkey to
    //    `nn_script` so that any validation logic checks pass.
    let mut stake_tx = create_dummy_tx(1, 1);
    stake_tx.output[0].script_pubkey = nn_script;

    // 2. Create the base unstake transaction using the provided metadata.
    let unstake_info = UnstakeInfo::new(
        header_aux.clone(),
        OutPoint::new(stake_tx.compute_txid(), 0).into(),
    );
    let unstake_tx = create_test_unstake_tx(&unstake_info);

    (stake_tx, unstake_tx)
}
