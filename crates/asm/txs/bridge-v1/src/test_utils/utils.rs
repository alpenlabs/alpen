use bitcoin::{ScriptBuf, Transaction, script::PushBytesBuf};
use strata_asm_common::TxInputRef;
use strata_l1_txfmt::ParseConfig;

use crate::test_utils::TEST_MAGIC_BYTES;

// Helper function to create tagged payload with custom parameters
pub fn create_tagged_payload(subprotocol_id: u8, tx_type: u8, aux_data: Vec<u8>) -> Vec<u8> {
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES);
    tagged_payload.push(subprotocol_id); // 1 byte subprotocol ID
    tagged_payload.push(tx_type); // 1 byte transaction type
    tagged_payload.extend_from_slice(&aux_data);
    tagged_payload
}

// Helper function to mutate transaction OP_RETURN output
pub fn mutate_op_return_output(tx: &mut Transaction, tagged_payload: Vec<u8>) {
    tx.output[0].script_pubkey =
        ScriptBuf::new_op_return(PushBytesBuf::try_from(tagged_payload).unwrap());
}

// Helper function to parse transaction
pub fn parse_tx(tx: &Transaction) -> TxInputRef<'_> {
    let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
    let tag_data = parser.try_parse_tx(tx).expect("Should parse transaction");
    TxInputRef::new(tx, tag_data)
}
