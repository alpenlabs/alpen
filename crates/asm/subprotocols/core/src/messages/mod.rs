//! Message processing for the Core subprotocol
//!
//! This module handles L1↔L2 message processing, validation, and forwarding.

pub(crate) mod l1_to_l2;
pub(crate) mod l2_to_l1;

// Re-export main message functions for convenience
pub(crate) use l2_to_l1::{extract_l2_to_l1_messages, validate_l2_to_l1_messages};
