//! Verification logic for the Core subprotocol
//!
//! This module contains all verification functionality including signature verification,
//! proof verification, and state transition validation.

pub(crate) mod proof;
pub(crate) mod signature;
pub(crate) mod state_transition;

// Re-export main verification functions for convenience
pub(crate) use proof::{construct_checkpoint_proof_public_parameters, verify_checkpoint_proof};
