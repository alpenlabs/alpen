use bitcoin::{Transaction, XOnlyPublicKey};
use strata_l1_txfmt::ParseConfig;
use strata_primitives::constants::RECOVER_DELAY;

use crate::{
    deposit_request::{DrtHeaderAux, create_deposit_request_locking_script},
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
};

pub fn create_test_deposit_request_tx(
    info: DrtHeaderAux,
    internal_key: XOnlyPublicKey,
) -> Transaction {
    let mut tx = create_dummy_tx(1, 2);
    let tag_data = info.encode_tag().unwrap();

    let parse_config = ParseConfig::new(*TEST_MAGIC_BYTES);
    let data = parse_config.encode_script_buf(&tag_data.as_ref()).unwrap();

    tx.output[0].script_pubkey = data;

    tx.output[1].script_pubkey =
        create_deposit_request_locking_script(info.recovery_pk(), internal_key, RECOVER_DELAY);

    tx
}
