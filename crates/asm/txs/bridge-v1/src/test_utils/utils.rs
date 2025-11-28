use bitcoin::Transaction;
use strata_asm_common::TxInputRef;
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::test_utils::TEST_MAGIC_BYTES;

// Helper function to mutate SPS 50 transaction auxiliary data
pub fn mutate_aux_data(tx: &mut Transaction, new_aux: Vec<u8>) {
    let config = ParseConfig::new(*TEST_MAGIC_BYTES);
    let td = config.try_parse_tx(tx).unwrap();
    let new_td = TagData::new(td.subproto_id(), td.tx_type(), new_aux).unwrap();
    let new_scriptbuf = config.encode_script_buf(&new_td.as_ref()).unwrap();
    tx.output[0].script_pubkey = new_scriptbuf
}

// Helper function to parse transaction
pub fn parse_tx(tx: &Transaction) -> TxInputRef<'_> {
    let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
    let tag_data = parser.try_parse_tx(tx).expect("Should parse transaction");
    TxInputRef::new(tx, tag_data)
}
