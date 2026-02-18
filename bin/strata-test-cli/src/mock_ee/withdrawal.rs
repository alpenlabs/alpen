//! Snark account withdrawal transaction builder.
//!
//! Builds a complete `RpcOLTransaction` JSON blob for `strata_submitTransaction`.
//! Pure computation — no network calls.

use ssz::Encode;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_msg_types::{WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_ID;
use strata_snark_acct_types::{
    LedgerRefs, OutputMessage, ProofState, UpdateOperationData, UpdateOutputs,
};

use crate::error::Error;

/// Builds an `RpcOLTransaction` JSON value for a snark account withdrawal.
///
/// The returned JSON matches the serde format expected by the `strata_submitTransaction` RPC.
pub(crate) fn build_snark_withdrawal_json(
    target: AccountId,
    seq_no: u64,
    inner_state: Hash,
    next_inbox_idx: u64,
    dest_bytes: Vec<u8>,
    amount: u64,
    fees: u32,
) -> Result<serde_json::Value, Error> {
    // Build withdrawal message data
    let withdrawal_msg_data = WithdrawalMsgData::new(fees, dest_bytes)
        .ok_or_else(|| Error::TxBuilder("destination descriptor too long".to_string()))?;

    let encoded_body = strata_codec::encode_to_vec(&withdrawal_msg_data)
        .map_err(|e| Error::TxBuilder(format!("failed to encode withdrawal msg data: {e}")))?;

    let owned_msg = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_body)
        .map_err(|e| Error::TxBuilder(format!("failed to create OwnedMsg: {e}")))?;

    let msg_payload = MsgPayload::new(
        BitcoinAmount::from_sat(amount),
        owned_msg.to_vec(),
    );

    let output_message = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg_payload);
    let outputs = UpdateOutputs::new(vec![], vec![output_message]);

    let proof_state = ProofState::new(inner_state, next_inbox_idx);

    let operation_data = UpdateOperationData::new(
        seq_no,
        proof_state,
        vec![],                  // no processed messages
        LedgerRefs::new_empty(), // no ledger references
        outputs,
        vec![],                  // no extra data
    );

    // SSZ-encode the operation data
    let ssz_bytes = operation_data.as_ssz_bytes();
    let ssz_hex = hex::encode(&ssz_bytes);

    // Build the target hex (plain hex, no 0x prefix — matches hex::serde format)
    let target_bytes: [u8; 32] = target.into();
    let target_hex = hex::encode(target_bytes);

    // Construct the JSON matching RpcOLTransaction serde format.
    // HexBytes/HexBytes32 use hex::serde which expects plain hex without 0x prefix.
    let json = serde_json::json!({
        "payload": {
            "type": "snark_account_update",
            "target": target_hex,
            "update_operation_encoded": ssz_hex,
            "update_proof": ""
        },
        "attachments": {
            "min_slot": null,
            "max_slot": null
        }
    });

    Ok(json)
}

#[cfg(test)]
mod tests {
    use ssz::Decode;
    use strata_snark_acct_types::UpdateOperationData;

    use super::*;

    #[test]
    fn test_build_snark_withdrawal_json_structure() {
        let target = AccountId::new([0u8; 32]);
        let inner_state = Hash::from([0u8; 32]);

        let json = build_snark_withdrawal_json(
            target,
            0,
            inner_state,
            0,
            b"bc1qexample".to_vec(),
            100_000_000,
            0,
        )
        .expect("should build json");

        // Verify top-level structure
        let payload = &json["payload"];
        assert_eq!(payload["type"], "snark_account_update");
        // target is 32 bytes = 64 hex chars, no 0x prefix
        assert_eq!(payload["target"].as_str().unwrap().len(), 64);
        // update_operation_encoded is non-empty hex
        assert!(!payload["update_operation_encoded"].as_str().unwrap().is_empty());
        // update_proof is empty (AlwaysAccept predicate)
        assert_eq!(payload["update_proof"], "");

        let attachments = &json["attachments"];
        assert!(attachments["min_slot"].is_null());
        assert!(attachments["max_slot"].is_null());
    }

    #[test]
    fn test_build_snark_withdrawal_ssz_roundtrip() {
        let mut target_bytes = [0u8; 32];
        target_bytes[31] = 0x42;
        let target = AccountId::new(target_bytes);
        let inner_state = Hash::from([1u8; 32]);

        let json = build_snark_withdrawal_json(
            target,
            5,
            inner_state,
            3,
            b"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_vec(),
            100_000_000,
            0,
        )
        .expect("should build json");

        // Extract SSZ bytes and decode them (plain hex, no 0x prefix)
        let ssz_hex = json["payload"]["update_operation_encoded"]
            .as_str()
            .unwrap();
        let ssz_bytes = hex::decode(ssz_hex).expect("valid hex");
        let decoded =
            UpdateOperationData::from_ssz_bytes(&ssz_bytes).expect("valid SSZ");

        // Verify decoded fields
        assert_eq!(decoded.seq_no(), 5);
        assert_eq!(decoded.new_proof_state().inner_state(), inner_state);
        assert_eq!(decoded.new_proof_state().next_inbox_msg_idx(), 3);
        assert_eq!(decoded.processed_messages().len(), 0);
        assert_eq!(decoded.ledger_refs().l1_header_refs().len(), 0);
        assert_eq!(decoded.outputs().transfers().len(), 0);
        assert_eq!(decoded.outputs().messages().len(), 1);

        // Verify the output message targets the bridge gateway
        let output_msg = &decoded.outputs().messages()[0];
        assert_eq!(output_msg.dest(), BRIDGE_GATEWAY_ACCT_ID);
        assert_eq!(
            output_msg.payload().value(),
            BitcoinAmount::from_sat(100_000_000)
        );
    }

    #[test]
    fn test_build_snark_withdrawal_target_hex() {
        let mut target_bytes = [0u8; 32];
        target_bytes[31] = 0x42;
        let target = AccountId::new(target_bytes);
        let inner_state = Hash::from([0u8; 32]);

        let json = build_snark_withdrawal_json(
            target,
            0,
            inner_state,
            0,
            b"bc1qexample".to_vec(),
            100_000_000,
            0,
        )
        .expect("should build json");

        let target_hex = json["payload"]["target"].as_str().unwrap();
        assert_eq!(
            target_hex,
            "0000000000000000000000000000000000000000000000000000000000000042"
        );
    }

    /// Verifies that the JSON output can be deserialized into the actual
    /// [`RpcOLTransaction`] type used by the RPC server, proving wire compatibility.
    #[test]
    fn test_build_snark_withdrawal_rpc_deserialize() {
        use strata_ol_rpc_types::RpcOLTransaction;

        let mut target_bytes = [0u8; 32];
        target_bytes[31] = 0x42;
        let target = AccountId::new(target_bytes);
        let inner_state = Hash::from([0u8; 32]);

        let json = build_snark_withdrawal_json(
            target,
            0,
            inner_state,
            0,
            b"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_vec(),
            100_000_000,
            0,
        )
        .expect("should build json");

        // Deserialize into the actual RPC type — this proves wire compatibility
        let _rpc_tx: RpcOLTransaction =
            serde_json::from_value(json).expect("should deserialize into RpcOLTransaction");
    }

    #[test]
    fn test_build_snark_withdrawal_dest_too_long() {
        let target = AccountId::new([0u8; 32]);
        let inner_state = Hash::from([0u8; 32]);

        // Descriptor longer than MAX_WITHDRAWAL_DESC_LEN (255)
        let long_dest = vec![0xAA; 256];
        let result = build_snark_withdrawal_json(
            target,
            0,
            inner_state,
            0,
            long_dest,
            100_000_000,
            0,
        );

        assert!(result.is_err());
    }
}
