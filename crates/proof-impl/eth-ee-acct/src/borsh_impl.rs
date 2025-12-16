//! Borsh serialization implementation for EthEeAcctProofOutput
//!
//! This module provides Borsh serialization by wrapping SSZ-encoded bytes.
//!
//! Current approach: Each field is serialized to SSZ individually, then wrapped in Borsh.
//!
//! Once all inner types have proper SSZ support, this can be simplified to:
//! - Derive SSZ Encode/Decode for the entire EthEeAcctProofOutput struct
//! - Serialize the whole struct to SSZ bytes in one call
//! - Wrap those single SSZ bytes in Borsh (much simpler than current per-field approach)

use std::io::{Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use ssz::{Decode, Encode};
use strata_ee_acct_types::UpdateExtraData;

use crate::EthEeAcctProofOutput;

impl BorshSerialize for EthEeAcctProofOutput {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize all fields as SSZ bytes wrapped in Borsh
        let prev_state_bytes = self.prev_state.as_ssz_bytes();
        BorshSerialize::serialize(&prev_state_bytes, writer)?;

        let final_state_bytes = self.final_state.as_ssz_bytes();
        BorshSerialize::serialize(&final_state_bytes, writer)?;

        BorshSerialize::serialize(&self.da_commitments, writer)?;

        let output_messages_bytes = self.output_messages.as_ssz_bytes();
        BorshSerialize::serialize(&output_messages_bytes, writer)?;

        let input_messages_bytes = self.input_messages.as_ssz_bytes();
        BorshSerialize::serialize(&input_messages_bytes, writer)?;

        let extra_data_bytes = encode_update_extra_data_ssz(&self.extra_data);
        BorshSerialize::serialize(&extra_data_bytes, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for EthEeAcctProofOutput {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // Deserialize all fields from SSZ bytes wrapped in Borsh
        let prev_state_bytes: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
        let prev_state = Decode::from_ssz_bytes(&prev_state_bytes)
            .map_err(|e| std::io::Error::other(format!("{:?}", e)))?;

        let final_state_bytes: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
        let final_state = Decode::from_ssz_bytes(&final_state_bytes)
            .map_err(|e| std::io::Error::other(format!("{:?}", e)))?;

        let da_commitments = BorshDeserialize::deserialize_reader(reader)?;

        let output_messages_bytes: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
        let output_messages = Decode::from_ssz_bytes(&output_messages_bytes)
            .map_err(|e| std::io::Error::other(format!("{:?}", e)))?;

        let input_messages_bytes: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
        let input_messages = Decode::from_ssz_bytes(&input_messages_bytes)
            .map_err(|e| std::io::Error::other(format!("{:?}", e)))?;

        let extra_data_bytes: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
        let extra_data =
            decode_update_extra_data_ssz(&extra_data_bytes).map_err(std::io::Error::other)?;

        Ok(Self {
            prev_state,
            final_state,
            da_commitments,
            output_messages,
            input_messages,
            extra_data,
        })
    }
}

/// Encode UpdateExtraData to SSZ bytes
/// TODO: Replace with actual SSZ encode once UpdateExtraData has SSZ support
fn encode_update_extra_data_ssz(extra_data: &UpdateExtraData) -> Vec<u8> {
    // Temporary manual serialization: [32 bytes hash][4 bytes u32][4 bytes u32]
    let mut bytes = Vec::with_capacity(40);
    bytes.extend_from_slice(extra_data.new_tip_blkid().as_ref());
    bytes.extend_from_slice(&extra_data.processed_inputs().to_le_bytes());
    bytes.extend_from_slice(&extra_data.processed_fincls().to_le_bytes());
    bytes
}

/// Decode UpdateExtraData from SSZ bytes
/// TODO: Replace with actual SSZ decode once UpdateExtraData has SSZ support
fn decode_update_extra_data_ssz(bytes: &[u8]) -> Result<UpdateExtraData, String> {
    if bytes.len() != 40 {
        return Err("Invalid UpdateExtraData byte length".to_string());
    }

    let mut new_tip_blkid = [0u8; 32];
    new_tip_blkid.copy_from_slice(&bytes[0..32]);

    let processed_inputs = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
    let processed_fincls = u32::from_le_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]);

    Ok(UpdateExtraData::new(
        new_tip_blkid,
        processed_inputs,
        processed_fincls,
    ))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_snark_acct_types::ProofState;

    use super::*;

    // Strategy for generating random ProofState
    fn arb_proof_state() -> impl Strategy<Value = ProofState> {
        (any::<[u8; 32]>(), any::<u64>()).prop_map(|(root, idx)| ProofState::new(root, idx))
    }

    // Strategy for generating random UpdateExtraData
    fn arb_update_extra_data() -> impl Strategy<Value = UpdateExtraData> {
        (any::<[u8; 32]>(), any::<u32>(), any::<u32>())
            .prop_map(|(blkid, inputs, fincls)| UpdateExtraData::new(blkid, inputs, fincls))
    }

    // Strategy for generating random EthEeAcctProofOutput
    fn arb_eth_ee_acct_proof_output() -> impl Strategy<Value = EthEeAcctProofOutput> {
        (
            arb_proof_state(),
            arb_proof_state(),
            prop::collection::vec(any::<[u8; 32]>(), 0..5),
            arb_update_extra_data(),
        )
            .prop_map(|(prev_state, final_state, da_commitments, extra_data)| {
                EthEeAcctProofOutput {
                    prev_state,
                    final_state,
                    da_commitments,
                    output_messages: vec![], // Empty for now (OutputMessage doesn't have Arbitrary)
                    input_messages: vec![],  // Empty for now (MessageEntry doesn't have Arbitrary)
                    extra_data,
                }
            })
    }

    proptest! {
        #[test]
        fn test_borsh_roundtrip_proptest(output in arb_eth_ee_acct_proof_output()) {
            // Serialize
            let serialized = borsh::to_vec(&output).expect("Failed to serialize");

            // Deserialize
            let deserialized: EthEeAcctProofOutput =
                borsh::from_slice(&serialized).expect("Failed to deserialize");

            // Verify round-trip for all fields
            prop_assert_eq!(
                deserialized.prev_state.inner_state(),
                output.prev_state.inner_state(),
                "prev_state inner_state mismatch"
            );
            prop_assert_eq!(
                deserialized.prev_state.next_inbox_msg_idx(),
                output.prev_state.next_inbox_msg_idx(),
                "prev_state next_inbox_msg_idx mismatch"
            );
            prop_assert_eq!(
                deserialized.final_state.inner_state(),
                output.final_state.inner_state(),
                "final_state inner_state mismatch"
            );
            prop_assert_eq!(
                deserialized.final_state.next_inbox_msg_idx(),
                output.final_state.next_inbox_msg_idx(),
                "final_state next_inbox_msg_idx mismatch"
            );
            prop_assert_eq!(
                deserialized.da_commitments,
                output.da_commitments,
                "da_commitments mismatch"
            );
            prop_assert_eq!(
                deserialized.extra_data.new_tip_blkid(),
                output.extra_data.new_tip_blkid(),
                "extra_data new_tip_blkid mismatch"
            );
            prop_assert_eq!(
                deserialized.extra_data.processed_inputs(),
                output.extra_data.processed_inputs(),
                "extra_data processed_inputs mismatch"
            );
            prop_assert_eq!(
                deserialized.extra_data.processed_fincls(),
                output.extra_data.processed_fincls(),
                "extra_data processed_fincls mismatch"
            );
        }

        #[test]
        fn test_update_extra_data_roundtrip_proptest(extra_data in arb_update_extra_data()) {
            // Encode
            let encoded = encode_update_extra_data_ssz(&extra_data);

            // Verify length
            prop_assert_eq!(encoded.len(), 40, "Encoded length should be 40 bytes");

            // Decode
            let decoded = decode_update_extra_data_ssz(&encoded)
                .expect("Failed to decode");

            // Verify round-trip
            prop_assert_eq!(
                decoded.new_tip_blkid(),
                extra_data.new_tip_blkid(),
                "new_tip_blkid mismatch"
            );
            prop_assert_eq!(
                decoded.processed_inputs(),
                extra_data.processed_inputs(),
                "processed_inputs mismatch"
            );
            prop_assert_eq!(
                decoded.processed_fincls(),
                extra_data.processed_fincls(),
                "processed_fincls mismatch"
            );
        }
    }

    #[test]
    fn test_update_extra_data_edge_cases() {
        // Test all zeros
        let zeros = UpdateExtraData::new([0u8; 32], 0, 0);
        let encoded = encode_update_extra_data_ssz(&zeros);
        let decoded = decode_update_extra_data_ssz(&encoded).expect("Failed to decode zeros");
        assert_eq!(decoded.new_tip_blkid(), zeros.new_tip_blkid());
        assert_eq!(decoded.processed_inputs(), zeros.processed_inputs());
        assert_eq!(decoded.processed_fincls(), zeros.processed_fincls());

        // Test max values
        let max_vals = UpdateExtraData::new([0xffu8; 32], u32::MAX, u32::MAX);
        let encoded = encode_update_extra_data_ssz(&max_vals);
        let decoded = decode_update_extra_data_ssz(&encoded).expect("Failed to decode max values");
        assert_eq!(decoded.new_tip_blkid(), max_vals.new_tip_blkid());
        assert_eq!(decoded.processed_inputs(), max_vals.processed_inputs());
        assert_eq!(decoded.processed_fincls(), max_vals.processed_fincls());
    }

    #[test]
    fn test_update_extra_data_invalid_length() {
        // Test with wrong length
        let invalid = vec![0u8; 39]; // One byte short
        assert!(
            decode_update_extra_data_ssz(&invalid).is_err(),
            "Should fail with invalid length"
        );

        let invalid = vec![0u8; 41]; // One byte too long
        assert!(
            decode_update_extra_data_ssz(&invalid).is_err(),
            "Should fail with invalid length"
        );
    }
}
