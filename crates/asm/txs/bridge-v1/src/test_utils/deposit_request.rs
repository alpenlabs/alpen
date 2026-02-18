use bitcoin::{Amount, Transaction, XOnlyPublicKey};
use strata_asm_txs_test_utils::{TEST_MAGIC_BYTES, create_dummy_tx};
use strata_l1_txfmt::ParseConfig;

use crate::deposit_request::{DrtHeaderAux, create_deposit_request_locking_script};

/// Creates a deposit request transaction with the correct outputs and the provided SPS50 metadata.
pub fn create_test_deposit_request_tx(
    header_aux: &DrtHeaderAux,
    internal_key: XOnlyPublicKey,
    deposit_amount: Amount,
    recovery_delay: u16,
) -> Transaction {
    // Create a tx with one input and two outputs.
    let mut tx = create_dummy_tx(1, 2);

    let tag_data = header_aux.build_tag_data();
    let sps50_script = ParseConfig::new(TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .expect("encoding SPS50 header script must succeed");

    // The first output is SPS 50 header.
    tx.output[0].script_pubkey = sps50_script;

    // Second output is the deposit-request locking script.
    tx.output[1].script_pubkey = create_deposit_request_locking_script(
        header_aux.recovery_pk(),
        internal_key,
        recovery_delay,
    );
    tx.output[1].value = deposit_amount;

    tx
}
