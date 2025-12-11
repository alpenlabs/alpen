//! Checkpoint transaction builders for testing.
//!
//! Provides utilities to construct SPS-50 envelope transactions containing
//! checkpoint payloads for use in integration tests.

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, XOnlyPublicKey,
    absolute::LockTime,
    blockdata::script,
    key::UntweakedKeypair,
    opcodes::{
        OP_FALSE,
        all::{OP_CHECKMULTISIG, OP_ENDIF, OP_IF},
    },
    script::PushBytesBuf,
    secp256k1::{SECP256K1, schnorr::Signature},
    taproot::{LeafVersion, TaprootBuilder},
    transaction::Version,
};
use rand::{RngCore, rngs::OsRng};
use ssz::Encode;
use strata_asm_proto_checkpoint_txs::{CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};
use strata_checkpoint_types_ssz::SignedCheckpointPayload;
use thiserror::Error;

/// Magic bytes for test transactions (matches test configuration).
pub const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

/// Errors that can occur when building checkpoint transactions.
#[derive(Debug, Error)]
pub enum CheckpointTxBuildError {
    /// Failed to build taproot spend info.
    #[error("failed to build taproot spend info")]
    TaprootBuild,

    /// SSZ serialization failed.
    #[error("SSZ serialization failed")]
    SszSerialize,
}

/// Result type for checkpoint transaction building.
pub type CheckpointTxBuildResult<T> = Result<T, CheckpointTxBuildError>;

/// Create a test checkpoint transaction with SPS-50 envelope format.
///
/// This creates a complete reveal transaction that can be parsed by the
/// checkpoint transaction parser. The transaction contains:
/// - Witness with taproot script spend containing the SSZ-encoded checkpoint
/// - OP_RETURN output with SPS-50 tag (magic bytes, subprotocol ID, tx type)
///
/// # Arguments
/// * `signed_checkpoint` - The signed checkpoint payload to embed
///
/// # Returns
/// A Bitcoin transaction suitable for testing checkpoint parsing
pub fn create_test_checkpoint_tx(
    signed_checkpoint: &SignedCheckpointPayload,
) -> CheckpointTxBuildResult<Transaction> {
    // Serialize the checkpoint to SSZ bytes
    let envelope_payload = signed_checkpoint.as_ssz_bytes();

    // Create the SPS-50 tag for OP_RETURN
    // Format: [MAGIC_BYTES][SUBPROTOCOL_ID][TX_TYPE]
    let mut tagged_payload = Vec::with_capacity(6);
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES);
    tagged_payload.push(CHECKPOINT_V0_SUBPROTOCOL_ID);
    tagged_payload.push(OL_STF_CHECKPOINT_TX_TYPE);

    create_checkpoint_reveal_tx(envelope_payload, tagged_payload)
}

/// Create a checkpoint reveal transaction with custom payloads.
///
/// This is a lower-level function that allows specifying the envelope
/// payload and SPS-50 tag separately.
pub fn create_checkpoint_reveal_tx(
    envelope_payload: Vec<u8>,
    sps50_tagged_payload: Vec<u8>,
) -> CheckpointTxBuildResult<Transaction> {
    // Generate a random keypair for the taproot commit
    let mut rand_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes)
        .map_err(|_| CheckpointTxBuildError::TaprootBuild)?;
    let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;

    // Build the reveal script containing the envelope
    let reveal_script = build_reveal_script(&public_key, &envelope_payload);

    // Create taproot spend info
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())
        .map_err(|_| CheckpointTxBuildError::TaprootBuild)?
        .finalize(SECP256K1, public_key)
        .map_err(|_| CheckpointTxBuildError::TaprootBuild)?;

    // Create witness with dummy signature, script, and control block
    let signature =
        Signature::from_slice(&[0u8; 64]).map_err(|_| CheckpointTxBuildError::TaprootBuild)?;
    let mut witness = Witness::new();
    witness.push(signature.as_ref());
    witness.push(reveal_script.clone());
    witness.push(
        taproot_spend_info
            .control_block(&(reveal_script, LeafVersion::TapScript))
            .ok_or(CheckpointTxBuildError::TaprootBuild)?
            .serialize(),
    );

    // Create the transaction
    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new_op_return(
                PushBytesBuf::try_from(sps50_tagged_payload)
                    .map_err(|_| CheckpointTxBuildError::TaprootBuild)?,
            ),
        }],
    };

    Ok(tx)
}

/// Build reveal script containing opcodes for key verification and envelope.
fn build_reveal_script(taproot_public_key: &XOnlyPublicKey, payload: &[u8]) -> ScriptBuf {
    let mut script_bytes = script::Builder::new()
        .push_x_only_key(taproot_public_key)
        .push_opcode(OP_CHECKMULTISIG)
        .into_script()
        .into_bytes();

    let envelope_script = build_checkpoint_envelope_script(payload);
    script_bytes.extend(envelope_script.into_bytes());

    ScriptBuf::from(script_bytes)
}

/// Build the envelope script containing the checkpoint payload.
///
/// Uses OP_FALSE OP_IF ... OP_ENDIF pattern to embed data in the script
/// without affecting execution.
pub fn build_checkpoint_envelope_script(payload: &[u8]) -> ScriptBuf {
    let mut builder = script::Builder::new()
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF);

    // Split payload into 520-byte chunks (max push size)
    for chunk in payload.chunks(520) {
        builder = builder
            .push_slice(PushBytesBuf::try_from(chunk.to_vec()).expect("chunk size within limits"));
    }

    builder = builder.push_opcode(OP_ENDIF);
    builder.into_script()
}

#[cfg(test)]
mod tests {
    use strata_asm_common::TxInputRef;
    use strata_asm_proto_checkpoint_txs::extract_signed_checkpoint_from_envelope;
    use strata_l1_txfmt::ParseConfig;

    use super::*;
    use crate::checkpoint::fixtures::CheckpointFixtures;

    #[test]
    fn test_create_checkpoint_tx_roundtrip() {
        // Generate test checkpoint
        let fixtures = CheckpointFixtures::new();
        let signed_checkpoint = fixtures.gen_signed_payload();

        // Create transaction
        let tx = create_test_checkpoint_tx(&signed_checkpoint).unwrap();

        // Verify transaction structure
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 1);

        // Verify witness has expected elements (signature, script, control block)
        assert_eq!(tx.input[0].witness.len(), 3);

        // Parse the SPS-50 tag
        let parse_config = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parse_config.try_parse_tx(&tx).expect("should parse tag");

        // Verify tag data
        assert_eq!(tag_data.subproto_id(), CHECKPOINT_V0_SUBPROTOCOL_ID);
        assert_eq!(tag_data.tx_type(), OL_STF_CHECKPOINT_TX_TYPE);

        // Parse checkpoint from envelope
        let tx_input = TxInputRef::new(&tx, tag_data);
        let parsed = extract_signed_checkpoint_from_envelope(&tx_input).unwrap();

        // Verify parsed checkpoint matches original
        assert_eq!(
            parsed.payload().epoch(),
            signed_checkpoint.payload().epoch()
        );
        assert_eq!(
            parsed.payload().batch_info(),
            signed_checkpoint.payload().batch_info()
        );
        assert_eq!(
            parsed.payload().transition(),
            signed_checkpoint.payload().transition()
        );
        assert_eq!(parsed.signature(), signed_checkpoint.signature());
    }

    #[test]
    fn test_envelope_script_chunking() {
        // Test with payload larger than 520 bytes
        let large_payload = vec![0xABu8; 1500];
        let script = build_checkpoint_envelope_script(&large_payload);

        // Script should be valid
        assert!(!script.is_empty());

        // Should contain OP_FALSE, OP_IF at start and OP_ENDIF at end
        let bytes = script.as_bytes();
        assert!(bytes.len() > large_payload.len()); // Script has overhead
    }

    #[test]
    fn test_multiple_epochs() {
        let fixtures = CheckpointFixtures::new();

        // Test epoch 0
        let signed_0 = fixtures.gen_signed_payload_for_epoch(0);
        let tx_0 = create_test_checkpoint_tx(&signed_0).unwrap();

        // Test epoch 1
        let signed_1 = fixtures.gen_signed_payload_for_epoch(1);
        let tx_1 = create_test_checkpoint_tx(&signed_1).unwrap();

        // Both should parse successfully
        let parse_config = ParseConfig::new(*TEST_MAGIC_BYTES);

        let tag_0 = parse_config.try_parse_tx(&tx_0).unwrap();
        let tx_input_0 = TxInputRef::new(&tx_0, tag_0);
        let parsed_0 = extract_signed_checkpoint_from_envelope(&tx_input_0).unwrap();
        assert_eq!(parsed_0.payload().epoch(), 0);

        let tag_1 = parse_config.try_parse_tx(&tx_1).unwrap();
        let tx_input_1 = TxInputRef::new(&tx_1, tag_1);
        let parsed_1 = extract_signed_checkpoint_from_envelope(&tx_input_1).unwrap();
        assert_eq!(parsed_1.payload().epoch(), 1);
    }
}
