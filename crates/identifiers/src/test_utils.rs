//! Test utilities and proptest strategies for identifier types.
//!
//! This module contains reusable test utilities and proptest strategies that are used
//! across multiple test modules to avoid code duplication.

#![allow(unreachable_pub, reason = "test utils module")]

use proptest::prelude::*;
use ssz_types::FixedBytes;

use crate::{
    Buf32, Buf64, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment,
    OLBlockId, Slot,
};

// =============================================================================
// Buffer strategies
// =============================================================================

/// Strategy for generating random [`Buf32`] values.
pub fn buf32_strategy() -> impl Strategy<Value = Buf32> {
    any::<[u8; 32]>().prop_map(Buf32::from)
}

/// Strategy for generating random [`Buf64`] values.
pub fn buf64_strategy() -> impl Strategy<Value = Buf64> {
    any::<[u8; 64]>().prop_map(Buf64::from)
}

// =============================================================================
// OL (Orchestration Layer) strategies
// =============================================================================

/// Strategy for generating random [`OLBlockId`] values.
pub fn ol_block_id_strategy() -> impl Strategy<Value = OLBlockId> {
    buf32_strategy().prop_map(OLBlockId::from)
}

/// Strategy for generating random [`Slot`] values.
pub fn slot_strategy() -> impl Strategy<Value = Slot> {
    any::<u64>().prop_map(Slot::from)
}

/// Strategy for generating random [`OLBlockCommitment`] values.
pub fn ol_block_commitment_strategy() -> impl Strategy<Value = OLBlockCommitment> {
    (slot_strategy(), ol_block_id_strategy())
        .prop_map(|(slot, blkid)| OLBlockCommitment::new(slot, blkid))
}

// =============================================================================
// Epoch strategies
// =============================================================================

/// Strategy for generating random [`Epoch`] values.
pub fn epoch_strategy() -> impl Strategy<Value = Epoch> {
    any::<Epoch>()
}

/// Strategy for generating random [`EpochCommitment`] values.
pub fn epoch_commitment_strategy() -> impl Strategy<Value = EpochCommitment> {
    (any::<u32>(), any::<u64>(), ol_block_id_strategy())
        .prop_map(|(epoch, last_slot, blkid)| EpochCommitment::new(epoch, last_slot, blkid))
}

// =============================================================================
// L1 (Bitcoin layer) strategies
// =============================================================================

/// Strategy for generating random [`L1BlockId`] values.
pub fn l1_block_id_strategy() -> impl Strategy<Value = L1BlockId> {
    buf32_strategy().prop_map(L1BlockId::from)
}

/// Strategy for generating random [`L1BlockCommitment`] values.
///
/// When the `bitcoin` feature is enabled, heights are validated against bitcoin
/// consensus rules and invalid heights are filtered out.
#[cfg(feature = "bitcoin")]
pub fn l1_block_commitment_strategy() -> impl Strategy<Value = L1BlockCommitment> {
    use bitcoin::absolute;
    (any::<u32>(), l1_block_id_strategy()).prop_filter_map(
        "valid bitcoin height",
        |(height, blkid)| {
            let height = absolute::Height::from_consensus(height).ok()?;
            Some(L1BlockCommitment::new(height, blkid))
        },
    )
}

/// Strategy for generating random [`L1BlockCommitment`] values.
#[cfg(not(feature = "bitcoin"))]
pub fn l1_block_commitment_strategy() -> impl Strategy<Value = L1BlockCommitment> {
    (any::<u32>(), l1_block_id_strategy())
        .prop_map(|(height, blkid)| L1BlockCommitment::new(height, blkid))
}

// =============================================================================
// SSZ strategies
// =============================================================================

/// Strategy for generating random [`FixedBytes<32>`] values.
pub fn fixed_bytes_32_strategy() -> impl Strategy<Value = FixedBytes<32>> {
    any::<[u8; 32]>().prop_map(FixedBytes::from)
}
