//! Anchor State Machine (ASM) types.

// Re-export bitcoin verification from btc-verification crate
// Re-export L1 block types and utilities from btc-types crate
pub use strata_btc_types::{L1HeaderRecord, L1TxRef};
pub use strata_btc_verification::{
    compute_block_hash, get_relative_difficulty_adjustment_height, BtcWork,
    HeaderVerificationState, L1VerificationError, TimestampStore,
};
