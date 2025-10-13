//! Application-Specific Module (ASM) types for the Strata rollup.
//!
//! This crate contains ASM-specific types that are independent of
//! the core primitives and state management layers.

// Re-export bitcoin verification from btc-verification crate
// Re-export L1 block types and utilities from btc-types crate
pub use strata_btc_types::{
    generate_l1_tx, DaCommitment, DepositInfo, DepositRequestInfo, DepositSpendInfo,
    L1BlockManifest, L1HeaderRecord, L1Tx, L1TxInclusionProof, L1TxProof, L1TxRef, L1WtxProof,
    ProtocolOperation, TxIdComputable, TxIdMarker, WithdrawalFulfillmentInfo, WtxIdMarker,
};
pub use strata_btc_verification::{
    compute_block_hash, get_relative_difficulty_adjustment_height, BtcWork,
    HeaderVerificationState, L1VerificationError, TimestampStore,
};
