use bitcoin::{Amount, Transaction, XOnlyPublicKey};
use strata_l1_txfmt::ParseConfig;
use strata_primitives::constants::RECOVER_DELAY;

use crate::{
    deposit_request::{DrtHeaderAux, create_deposit_request_locking_script},
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
};

pub fn create_test_deposit_request_tx(
    header_aux: &DrtHeaderAux,
    internal_key: XOnlyPublicKey,
    deposit_amount: Amount,
) -> Transaction {
    // Create a dummy tx with one input and two outputs
    let mut tx = create_dummy_tx(1, 2);

    let tag_data = header_aux.build_tag_data().unwrap();
    let sps50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .unwrap();

    // The first output is SPS 50 header
    tx.output[0].script_pubkey = sps50_script;

    tx.output[1].script_pubkey = create_deposit_request_locking_script(
        header_aux.recovery_pk(),
        internal_key,
        RECOVER_DELAY,
    );
    tx.output[1].value = deposit_amount;

    tx
}
