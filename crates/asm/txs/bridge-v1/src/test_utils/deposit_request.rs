use bitcoin::{
    ScriptBuf, Transaction, XOnlyPublicKey,
    opcodes::all::{OP_CHECKSIGVERIFY, OP_CSV},
    script::Builder,
    secp256k1::Secp256k1,
    taproot::TaprootBuilder,
};
use strata_l1_txfmt::{ParseConfig, TagData};
use strata_primitives::constants::RECOVER_DELAY;

use crate::{
    BRIDGE_V1_SUBPROTOCOL_ID,
    constants::DEPOSIT_REQUEST_TX_TYPE,
    deposit_request::DepositRequestAuxData,
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
};

pub fn create_test_deposit_request_tx(
    info: DepositRequestAuxData,
    internal_key: XOnlyPublicKey,
) -> Transaction {
    let mut tx = create_dummy_tx(1, 2);

    let mut aux_data = Vec::new();
    aux_data.extend_from_slice(&info.recovery_pk);
    aux_data.extend_from_slice(&info.ee_address);

    let tag_data =
        TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_REQUEST_TX_TYPE, aux_data).unwrap();

    let parse_config = ParseConfig::new(*TEST_MAGIC_BYTES);
    let data = parse_config.encode_script_buf(&tag_data.as_ref()).unwrap();

    tx.output[0].script_pubkey = data;

    tx.output[1].script_pubkey = create_takeback_taproot_output(&info.recovery_pk, internal_key);

    tx
}

fn create_takeback_taproot_output(
    recovery_pk: &[u8; 32],
    internal_key: XOnlyPublicKey,
) -> ScriptBuf {
    let secp = Secp256k1::new();

    let tapscript = Builder::new()
        .push_slice(recovery_pk)
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_int(RECOVER_DELAY as i64)
        .push_opcode(OP_CSV)
        .into_script();

    let taproot_builder = TaprootBuilder::new()
        .add_leaf(0, tapscript)
        .expect("valid tapscript leaf");
    let spend_info = taproot_builder
        .finalize(&secp, internal_key)
        .expect("taproot finalization should succeed");

    ScriptBuf::new_p2tr(&secp, internal_key, spend_info.merkle_root())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::BRIDGE_V1_SUBPROTOCOL_ID;

    /// Deterministic internal key for tests (BIP-340 generator point x-only pubkey)
    const TEST_INTERNAL_KEY_BYTES: [u8; 32] = [
        0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE, 0x87, 0x0B,
        0x07, 0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81, 0x5B, 0x16, 0xF8,
        0x17, 0x98,
    ];

    #[test]
    fn sets_takeback_output_to_taproot_script() {
        let info = DepositRequestAuxData {
            recovery_pk: [0x02; 32],
            ee_address: vec![0xAB; 20],
        };

        let internal_key =
            XOnlyPublicKey::from_slice(&TEST_INTERNAL_KEY_BYTES).expect("valid x-only pubkey");

        let tx = create_test_deposit_request_tx(info.clone(), internal_key);

        let expected_script = create_takeback_taproot_output(&info.recovery_pk, internal_key);
        assert_eq!(
            tx.output[1].script_pubkey, expected_script,
            "DRT output[1] should be the taproot takeback output"
        );
    }

    #[test]
    fn op_return_contains_expected_tagged_data() {
        let info = DepositRequestAuxData {
            recovery_pk: [0x05; 32],
            ee_address: vec![0xCD; 32],
        };
        let internal_key =
            XOnlyPublicKey::from_slice(&TEST_INTERNAL_KEY_BYTES).expect("valid x-only pubkey");
        let tx = create_test_deposit_request_tx(info.clone(), internal_key);

        let parsed = ParseConfig::new(*TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("should parse SPS-50 header");

        assert_eq!(parsed.subproto_id(), BRIDGE_V1_SUBPROTOCOL_ID);
        assert_eq!(parsed.tx_type(), DEPOSIT_REQUEST_TX_TYPE);

        let mut expected_aux = Vec::new();
        expected_aux.extend_from_slice(&info.recovery_pk);
        expected_aux.extend_from_slice(&info.ee_address);
        assert_eq!(parsed.aux_data(), expected_aux.as_slice());
    }
}
