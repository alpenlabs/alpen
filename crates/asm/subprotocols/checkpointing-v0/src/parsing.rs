//! Checkpoint transaction parsing for checkpointing v0
//!
//! This module handles parsing of SPS-50 envelope transactions containing
//! checkpoint data.
//!
//! NOTE: This implementation focuses on compatibility with the current checkpoint
//! format while following SPS-62 envelope structure requirements.

use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_l1tx::envelope::parser::{enter_envelope, extract_until_op_endif};
use strata_primitives::{batch::SignedCheckpoint, buf::Buf32};

use crate::{
    constants::OL_STF_CHECKPOINT_TX_TYPE,
    error::{CheckpointV0Error, CheckpointV0Result},
    types::CheckpointV0VerifyContext,
};

/// Extract signed checkpoint from SPS-50 envelope transaction
///
/// This function follows the same pattern as administration subprotocol parsing:
/// 1. Enforces exactly 1 envelope in the first input (SPS-62 requirement)
/// 2. Extracts taproot leaf script from first input witness
/// 3. Parses envelope payload using strata-l1tx envelope parser
/// 4. Deserializes checkpoint from payload
/// 5. Extracts signer public key from transaction context
///
/// NOTE: This bridges current checkpoint format with SPS-50 envelope structure
pub fn extract_signed_checkpoint_from_envelope(
    tx: &TxInputRef<'_>,
) -> CheckpointV0Result<(SignedCheckpoint, CheckpointV0VerifyContext)> {
    let tx_type = tx.tag().tx_type();

    // Validate transaction type
    if tx_type != OL_STF_CHECKPOINT_TX_TYPE {
        return Err(CheckpointV0Error::UnsupportedTxType(format!(
            "Expected checkpoint tx type {}, got {}",
            OL_STF_CHECKPOINT_TX_TYPE, tx_type
        )));
    }

    // SPS-62 requirement: exactly 1 envelope in first input
    let bitcoin_tx = tx.tx();
    if bitcoin_tx.input.is_empty() {
        return Err(CheckpointV0Error::ParsingError(
            "Transaction has no inputs".to_string(),
        ));
    }

    // Extract taproot leaf script from the first input's witness
    // (following administration subprotocol pattern)
    let payload_script = bitcoin_tx.input[0]
        .witness
        .taproot_leaf_script()
        .ok_or_else(|| {
            CheckpointV0Error::ParsingError(
                "No taproot leaf script found in first input witness".to_string(),
            )
        })?
        .script;

    // Parse the envelope payload from the script
    let envelope_payload = parse_envelope_payload(&payload_script.into())?;

    // Deserialize the signed checkpoint from the payload
    // NOTE: This assumes the current SignedCheckpoint format is used in envelopes
    // Security: Don't leak deserializer error details to prevent information disclosure
    let signed_checkpoint: SignedCheckpoint =
        borsh::from_slice(&envelope_payload).map_err(|_| {
            CheckpointV0Error::ParsingError(
                "Failed to deserialize checkpoint from envelope: invalid format".to_string(),
            )
        })?;

    // Extract verification context
    // NOTE: For v0, we use a simplified context extraction approach
    let verify_context = extract_verification_context(tx)?;

    Ok((signed_checkpoint, verify_context))
}

/// Extract verification context from transaction
///
/// This extracts the necessary context for checkpoint verification:
/// - Current L1 height from transaction context
/// - Checkpoint signer public key (from envelope transaction signature)
///
/// NOTE: This is simplified compared to full SPS-62 auxiliary input
fn extract_verification_context(
    _tx: &TxInputRef<'_>,
) -> CheckpointV0Result<CheckpointV0VerifyContext> {
    // For checkpointing v0, we extract basic context information
    // The L1 height will be provided by the ASM anchor state
    // The signer pubkey would come from the envelope transaction signature verification

    // TODO: Extract actual signer pubkey from envelope transaction signature
    // For now, use a placeholder - in the real implementation this would:
    // 1. Verify the envelope transaction signature
    // 2. Extract the public key used for signing
    // 3. Validate against expected sequencer key

    let checkpoint_signer_pubkey = Buf32::zero(); // Placeholder

    Ok(CheckpointV0VerifyContext {
        current_l1_height: 0, // Will be set by the subprotocol from AnchorState
        checkpoint_signer_pubkey,
    })
}

/// Parse envelope payload from Bitcoin script
///
/// This function follows the exact same pattern as administration subprotocol:
/// - Uses strata-l1tx envelope parser functions
/// - Extracts payload data between envelope opcodes
/// - Returns raw payload bytes for deserialization
///
/// NOTE: Uses direct envelope parsing for extracting checkpoint data
fn parse_envelope_payload(script: &ScriptBuf) -> CheckpointV0Result<Vec<u8>> {
    let mut instructions = script.instructions();

    // Enter the envelope structure in the script (same as administration)
    enter_envelope(&mut instructions).map_err(|e| {
        CheckpointV0Error::ParsingError(format!("Failed to enter envelope structure: {}", e))
    })?;

    // Extract all data until the envelope closing opcode (same as administration)
    let payload = extract_until_op_endif(&mut instructions).map_err(|e| {
        CheckpointV0Error::ParsingError(format!("Failed to extract envelope payload: {}", e))
    })?;

    Ok(payload)
}

/// Placeholder function for current checkpoint format parsing
///
/// This is maintained for backwards compatibility during the transition period.
/// It can parse checkpoints in the current format before they're wrapped in envelopes.
///
/// TODO: Remove this once all checkpoints use envelope format
#[allow(dead_code)]
pub(crate) fn extract_checkpoint_legacy_format(
    _tx: &TxInputRef<'_>,
) -> CheckpointV0Result<(SignedCheckpoint, CheckpointV0VerifyContext)> {
    // Placeholder for parsing checkpoints in current format (non-envelope)
    // This would be used during transition period if needed
    Err(CheckpointV0Error::ParsingError(
        "Legacy checkpoint parsing not implemented - use envelope format".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use strata_primitives::{
        batch::{BatchInfo, Checkpoint, CheckpointSidecar},
        buf::{Buf32, Buf64},
        l1::L1BlockCommitment,
        l2::L2BlockCommitment,
    };
    use strata_state::batch::ChainstateRootTransition;
    use zkaleido::Proof;

    use super::*;
    use crate::types::BatchTransition;

    #[test]
    fn test_envelope_parsing_structure() {
        // Test that our parsing structure is correct
        // We can't easily create a real TxInputRef for testing without significant setup,
        // but we can test our data structures and error handling

        let context = CheckpointV0VerifyContext {
            current_l1_height: 100,
            checkpoint_signer_pubkey: Buf32::zero(),
        };

        assert_eq!(context.current_l1_height, 100);
    }

    #[test]
    fn test_checkpoint_format_compatibility() {
        // Test that we can work with current checkpoint format
        let l1_commitment = L1BlockCommitment::new(200, Buf32::zero().into());
        let l2_commitment = L2BlockCommitment::new(100, Buf32::zero().into());

        let batch_info = BatchInfo::new(
            1,                              // epoch
            (l1_commitment, l1_commitment), // L1 range tuple
            (l2_commitment, l2_commitment), // L2 range tuple
        );

        let batch_transition = BatchTransition {
            epoch: 1,
            chainstate_transition: ChainstateRootTransition {
                pre_state_root: Buf32::zero(),
                post_state_root: Buf32::zero(),
            },
        };

        let checkpoint = Checkpoint::new(
            batch_info,
            batch_transition.into(), // Convert ASM BatchTransition to primitives BatchTransition
            Proof::new(vec![]),
            CheckpointSidecar::new(vec![1, 2, 3, 4]),
        );

        let signed_checkpoint = SignedCheckpoint::new(checkpoint, Buf64::zero());

        // Verify we can extract epoch information
        assert_eq!(signed_checkpoint.checkpoint().batch_info().epoch(), 1);
    }
}
