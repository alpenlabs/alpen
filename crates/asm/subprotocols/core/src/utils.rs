use strata_asm_common::{L2ToL1Msg, Message};
use strata_primitives::{buf::Buf32, hash};

use crate::error::*;

/// Computes a rolling hash over L1→L2 message commitments
///
/// This function implements a rolling hash algorithm that processes L1 block
/// commitments in sequence, maintaining a running hash state that can be
/// incrementally updated as new blocks are processed.
///
/// # Arguments
/// * `l1_commitments` - Vector of L1 block commitments to hash
/// * `start_height` - Starting L1 block height for the range
/// * `end_height` - Ending L1 block height for the range
///
/// # Returns
/// The rolling hash commitment or an error if validation fails
pub(crate) fn compute_rolling_hash(
    l1_commitments: Vec<Buf32>,
    start_height: u64,
    end_height: u64,
) -> Result<Buf32> {
    // Validate height range
    if start_height > end_height {
        return Err(CoreError::InvalidL1BlockHeight {
            reason: format!("start height {start_height} cannot be greater than end height {end_height}"),
        });
    }

    // Validate range consistency

    if !(start_height <= end_height
        && l1_commitments.len() == (end_height - start_height + 1) as usize)
    {
        return Err(CoreError::L1ToL2RangeMismatch);
    }

    compute_rolling_hash_from_range(l1_commitments, start_height, end_height)
}

/// Computes rolling hash from a validated L1BlockRange
///
/// This implements the actual rolling hash algorithm:
/// rolling_hash = SHA256(rolling_hash || block_commitment)
/// starting with an initial seed based on the range parameters.
fn compute_rolling_hash_from_range(
    l1_commitments: Vec<Buf32>,
    start_height: u64,
    end_height: u64,
) -> Result<Buf32> {
    // Initialize with range metadata
    let mut rolling_state = Vec::new();
    rolling_state.extend_from_slice(&start_height.to_be_bytes());
    rolling_state.extend_from_slice(&end_height.to_be_bytes());

    // Initial hash of the range metadata
    let mut current_hash = hash::raw(&rolling_state);

    // Empty range case
    if l1_commitments.is_empty() {
        return Ok(current_hash);
    }

    // Rolling hash computation: hash(prev_hash || commitment) for each block
    for commitment in &l1_commitments {
        let mut data = Vec::with_capacity(64); // 32 bytes hash + 32 bytes commitment
        data.extend_from_slice(current_hash.as_ref());
        data.extend_from_slice(commitment.as_ref());
        current_hash = hash::raw(&data);
    }

    Ok(current_hash)
}

/// Validates the structure and content of OL→ASM messages
///
/// # Arguments
/// * `messages` - Vector of OLToASMMessage to validate
///
/// # Returns
/// Result indicating validation success or specific error
pub(crate) fn validate_l2_to_l1_messages(messages: &[L2ToL1Msg]) -> Result<()> {
    for (idx, msg) in messages.iter().enumerate() {
        // Validate that the message can be decoded
        match msg.decode() {
            Ok(decoded) => {
                // Validate message type is within expected range
                if decoded.ty() > Message::MAX_TYPE {
                    return Err(CoreError::MissingRequiredFieldInL2ToL1Msg {
                        index: idx,
                        field: "valid message type".into(),
                    });
                }
                
                // TODO: Add message type-specific validation once message types are defined
                // For example:
                // - Type 0x01 might be withdrawal messages
                // - Type 0x02 might be upgrade messages
                // Each type would have its own validation logic
            }
            Err(_) => {
                return Err(CoreError::MissingRequiredFieldInL2ToL1Msg {
                    index: idx,
                    field: "valid message encoding".into(),
                });
            }
        }
    }

    Ok(())
}
