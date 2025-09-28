//! State transition validation
//!
//! Validates that state transitions follow protocol rules and maintain consistency.

use strata_checkpoint_types::Checkpoint;

use crate::{CoreOLState, error::*};

/// Validates that a checkpoint represents a valid state transition
///
/// This function ensures that the new checkpoint follows proper progression
/// rules for epochs, block heights, and other state invariants.
///
/// # Arguments
/// * `current_state` - The current Core subprotocol state
/// * `checkpoint` - The new checkpoint to validate
///
/// # Returns
/// Result indicating if the state transition is valid
pub(crate) fn validate_state_transition(
    current_state: &CoreOLState,
    checkpoint: &Checkpoint,
) -> Result<()> {
    let prev_epoch_summary = &current_state.verified_checkpoint;
    let new_batch_info = checkpoint.batch_info();

    // Validate epoch progression
    let expected_epoch = (prev_epoch_summary.epoch() + 1) as u32;
    let actual_epoch = new_batch_info.epoch() as u32;
    if actual_epoch != expected_epoch {
        return Err(CoreError::InvalidEpoch {
            expected: expected_epoch,
            actual: actual_epoch,
        });
    }

    let new_l2_terminal = *new_batch_info.final_l2_block();

    // Validate L2 block slot progression
    let prev_slot = prev_epoch_summary.terminal().slot();
    let new_slot = new_l2_terminal.slot();
    if new_slot <= prev_slot {
        return Err(CoreError::InvalidL2BlockSlot {
            prev_slot,
            new_slot,
        });
    }

    // Validate L1 block height progression
    let prev_l1_height = prev_epoch_summary.new_l1().height();
    let new_l1_height = new_batch_info.final_l1_block().height();
    if new_l1_height <= prev_l1_height {
        return Err(CoreError::InvalidL1BlockHeight(format!(
            "new L1 height {new_l1_height} must be greater than previous height {prev_l1_height}"
        )));
    }

    // [PLACE_HOLDER] => Validate auxiliary data related things

    Ok(())
}
