//! Message processing for the Core subprotocol
//!
//! This module handles L1↔L2 message processing, validation, and forwarding.

mod l1_to_l2;
mod l2_to_l1;

// Re-export main message functions for convenience
pub(crate) use l1_to_l2::{compute_rolling_hash, validate_l1_to_l2_messages};
pub(crate) use l2_to_l1::{extract_l2_to_l1_messages, validate_l2_to_l1_messages};
