//! Verification logic for the Core subprotocol
//!
//! This module contains all verification functionality including signature verification,
//! proof verification, and state transition validation.

mod proof;
mod signature;
mod state_transition;

// Re-export main verification functions for convenience
pub(crate) use proof::{construct_checkpoint_proof_public_parameters, verify_checkpoint_proof};
pub(crate) use signature::verify_checkpoint_signature;
pub(crate) use state_transition::validate_state_transition;
