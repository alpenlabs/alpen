//! Checkpoint subprotocol test utilities
//!
//! Provides helpers for testing checkpoint subprotocol state and inter-subprotocol messaging.
//!
//! # Example
//!
//! ```ignore
//! use harness::test_harness::create_test_harness;
//! use harness::checkpoint::CheckpointExt;
//!
//! let harness = create_test_harness().await?;
//! let state = harness.checkpoint_state()?;
//! ```

use strata_asm_common::{AnchorState, Subprotocol};
use strata_asm_proto_checkpoint_v0::{CheckpointV0Subproto, CheckpointV0VerifierState};

use super::test_harness::AsmTestHarness;

/// Checkpoint subprotocol ID per SPS-50.
pub const SUBPROTOCOL_ID: u8 = 1;

/// Extract checkpoint subprotocol state from AnchorState.
pub fn extract_checkpoint_state(
    anchor_state: &AnchorState,
) -> anyhow::Result<CheckpointV0VerifierState> {
    let section = anchor_state
        .find_section(CheckpointV0Subproto::ID)
        .ok_or_else(|| anyhow::anyhow!("Checkpoint section not found"))?;
    let checkpoint_state = section.try_to_state::<CheckpointV0Subproto>()?;
    Ok(checkpoint_state)
}

// ============================================================================
// Checkpoint Extension Trait
// ============================================================================

/// Extension trait for checkpoint subprotocol operations on the test harness.
///
/// This trait provides checkpoint-specific convenience methods while keeping
/// the core harness infrastructure-focused.
pub trait CheckpointExt {
    /// Get checkpoint subprotocol state.
    fn checkpoint_state(&self) -> anyhow::Result<CheckpointV0VerifierState>;
}

impl CheckpointExt for AsmTestHarness {
    fn checkpoint_state(&self) -> anyhow::Result<CheckpointV0VerifierState> {
        let (_, asm_state) = self
            .get_latest_asm_state()?
            .ok_or_else(|| anyhow::anyhow!("No ASM state available"))?;
        extract_checkpoint_state(asm_state.state())
    }
}
