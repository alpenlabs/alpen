//! Snark account withdrawal transaction builder.
//!
//! Builds a complete `RpcOLTransaction` JSON blob for `strata_submitTransaction`.
//! Pure computation — no network calls.

use anyhow::Context;
use k256::schnorr::{signature::Signer, Signature, SigningKey};
use ssz::Encode;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MessageEntry, MsgPayload};
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_msg_types::{WithdrawalMsgData, WITHDRAWAL_MSG_TYPE_ID};
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_ID;
use strata_snark_acct_types::{
    LedgerRefs, OutputMessage, ProofState, Seqno, UpdateOperationData, UpdateOutputs,
    UpdateProofPubParams,
};

/// Deterministic BIP-340 signing key whose verifying key matches the
/// `bip340-schnorr-test` alpen-acct predicate baked into functional-test
/// rollup params. Must stay in sync with
/// `strata_proofimpl_alpen_acct::program::test_signing_key`.
const ALPEN_ACCT_TEST_SK_BYTES: [u8; 32] = [0x02u8; 32];

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
) -> anyhow::Result<serde_json::Value> {
    // Build withdrawal message data
    let withdrawal_msg_data =
        WithdrawalMsgData::new(fees, dest_bytes, 0).context("destination descriptor too long")?;

    let encoded_body = strata_codec::encode_to_vec(&withdrawal_msg_data)
        .context("failed to encode withdrawal msg data")?;

    let owned_msg =
        OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_body).context("failed to create OwnedMsg")?;

    let msg_payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(amount), owned_msg.to_vec())
        .expect("withdrawal message payload bytes must fit within SSZ max length");

    let output_message = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg_payload);
    let outputs = UpdateOutputs::new(vec![], vec![output_message]);

    let proof_state = ProofState::new(inner_state, next_inbox_idx);

    let operation_data = UpdateOperationData::new(
        seq_no,
        proof_state.clone(),
        vec![],                  // no processed messages
        LedgerRefs::new_empty(), // no ledger references
        outputs.clone(),
        vec![], // no extra data
    );

    // SSZ-encode the operation data
    let ssz_bytes = operation_data.as_ssz_bytes();
    let ssz_hex = hex::encode(&ssz_bytes);

    // Compute the predicate claim the OL will reconstruct in
    // `snark_acct_sys::compute_update_claim`, then sign it with the
    // deterministic alpen-acct test signing key so the Bip340Schnorr
    // predicate on the genesis snark account accepts the witness.
    //
    // The mock withdrawal does not advance the snark account's inner state
    // or inbox index, so `cur_state == new_state == proof_state`.
    let claim_ssz = sign_claim_ssz(Seqno::new(seq_no), &proof_state, &proof_state, &outputs);
    let signature = bip340_test_sign(&claim_ssz);
    let update_proof_hex = hex::encode(signature);

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
            "update_proof": update_proof_hex,
        },
        "constraints": {
            "min_slot": null,
            "max_slot": null
        }
    });

    Ok(json)
}

/// Reconstructs the `UpdateProofPubParams` claim the OL builds in
/// `snark_acct_sys::compute_update_claim` and returns its SSZ encoding.
fn sign_claim_ssz(
    seq_no: Seqno,
    cur_state: &ProofState,
    new_state: &ProofState,
    outputs: &UpdateOutputs,
) -> Vec<u8> {
    let pub_params = UpdateProofPubParams::new(
        seq_no,
        cur_state.clone(),
        new_state.clone(),
        Vec::<MessageEntry>::new(),
        LedgerRefs::new_empty(),
        outputs.clone(),
        Vec::new(),
    );
    pub_params.as_ssz_bytes()
}

/// Signs `msg` with the alpen-acct test BIP-340 Schnorr signing key.
///
/// The 64-byte signature satisfies the `Bip340Schnorr` predicate whose
/// 32-byte x-only public key is derived from `ALPEN_ACCT_TEST_SK_BYTES`.
fn bip340_test_sign(msg: &[u8]) -> [u8; 64] {
    let sk = SigningKey::from_bytes(&ALPEN_ACCT_TEST_SK_BYTES)
        .expect("hard-coded alpen-acct test signing key bytes must be valid");
    let sig: Signature = sk.sign(msg);
    sig.to_bytes()
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
        assert!(!payload["update_operation_encoded"]
            .as_str()
            .unwrap()
            .is_empty());
        // update_proof is a 64-byte BIP-340 Schnorr signature (128 hex chars).
        let update_proof_hex = payload["update_proof"].as_str().unwrap();
        assert_eq!(
            update_proof_hex.len(),
            128,
            "expected 64-byte Schnorr signature hex"
        );

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
        let seq_no = 5;

        let json = build_snark_withdrawal_json(
            target,
            seq_no,
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
        let decoded = UpdateOperationData::from_ssz_bytes(&ssz_bytes).expect("valid SSZ");

        // Verify decoded fields
        assert_eq!(decoded.seq_no(), seq_no);
        assert_eq!(decoded.new_proof_state().inner_state(), inner_state);
        assert_eq!(decoded.new_proof_state().next_inbox_msg_idx(), 3);
        assert_eq!(decoded.processed_messages().len(), 0);
        assert_eq!(decoded.ledger_refs().l1_block_refs().len(), 0);
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

    /// Locks in that the hard-coded signing key produces the exact
    /// verifying key wired into functional-test params and the
    /// `bip340-schnorr-test` datatool variant. If this fails, either the
    /// SK bytes here drifted from `strata_proofimpl_alpen_acct` or the hex
    /// pinned in `entry.py` / the ASM params defaults drifted.
    #[test]
    fn test_alpen_acct_test_signing_key_pubkey_matches_pinned_hex() {
        let sk = SigningKey::from_bytes(&ALPEN_ACCT_TEST_SK_BYTES).unwrap();
        let pubkey_hex = hex::encode(sk.verifying_key().to_bytes());
        assert_eq!(
            pubkey_hex, "4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766",
            "alpen-acct test signing key drifted from the pinned predicate hex"
        );
    }

    /// Checks that the signature produced for a withdrawal verifies against
    /// the alpen-acct test predicate using the same claim the OL reconstructs.
    #[test]
    fn test_build_snark_withdrawal_proof_verifies_under_test_predicate() {
        use k256::schnorr::{signature::Verifier, Signature, VerifyingKey};

        let mut target_bytes = [0u8; 32];
        target_bytes[31] = 0x42;
        let target = AccountId::new(target_bytes);
        let inner_state = Hash::from([1u8; 32]);
        let seq_no = 5;

        let json = build_snark_withdrawal_json(
            target,
            seq_no,
            inner_state,
            3,
            b"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_vec(),
            100_000_000,
            0,
        )
        .expect("should build json");

        // Rebuild the claim the OL would compute. The mock withdrawal does
        // not advance state, so cur_state == new_state.
        let proof_state = ProofState::new(inner_state, 3);
        let withdrawal_msg_data =
            WithdrawalMsgData::new(0, b"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_vec(), 0)
                .unwrap();
        let encoded_body = strata_codec::encode_to_vec(&withdrawal_msg_data).unwrap();
        let owned_msg = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_body).unwrap();
        let msg_payload =
            MsgPayload::from_bytes(BitcoinAmount::from_sat(100_000_000), owned_msg.to_vec())
                .expect("withdrawal message payload bytes must fit within SSZ max length");
        let output_message = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg_payload);
        let outputs = UpdateOutputs::new(vec![], vec![output_message]);
        let claim_ssz = sign_claim_ssz(Seqno::new(seq_no), &proof_state, &proof_state, &outputs);

        let proof_hex = json["payload"]["update_proof"].as_str().unwrap();
        let proof_bytes = hex::decode(proof_hex).unwrap();
        let signature = Signature::try_from(proof_bytes.as_slice()).unwrap();
        let vk = VerifyingKey::from_bytes(
            &hex::decode("4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766")
                .unwrap(),
        )
        .unwrap();

        vk.verify(&claim_ssz, &signature)
            .expect("withdrawal signature must verify under the alpen-acct test predicate");
    }

    #[test]
    fn test_build_snark_withdrawal_dest_too_long() {
        let target = AccountId::new([0u8; 32]);
        let inner_state = Hash::from([0u8; 32]);

        // Descriptor longer than MAX_WITHDRAWAL_DESC_LEN (255)
        let long_dest = vec![0xAA; 256];
        let result =
            build_snark_withdrawal_json(target, 0, inner_state, 0, long_dest, 100_000_000, 0);

        assert!(result.is_err());
    }
}
