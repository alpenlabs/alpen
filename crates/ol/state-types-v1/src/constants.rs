//! Protocol constants.

use strata_identifiers::{Hash, L1_HEIGHT_MMR_PREFILL_LEAF};

/// [`L1_HEIGHT_MMR_PREFILL_LEAF`] as a [`tyalias@Hash`], the sentinel leaf that prefills the OL's
/// L1-height-indexed MMRs (ASM manifests and L1 block refs) for heights at or before genesis.
///
/// The MMRs are height-indexed, so positions `0..=genesis_l1_height` hold the sentinel and
/// height `h` lands at index `h`. Using the shared upstream leaf keeps the OL prefill and the
/// ASM manifest MMR on the same bytes.
pub const MMR_SENTINEL_DUMMY_LEAF_HASH: Hash = Hash::new(L1_HEIGHT_MMR_PREFILL_LEAF);
