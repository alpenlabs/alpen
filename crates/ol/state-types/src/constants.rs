//! Protocol constants.

/// Sentinel leaf value used to prefill the ASM manifests MMR for L1 heights at
/// or before ASM genesis.
///
/// The MMR is height-indexed.  Positions for blocks at heights
/// `0..=genesis_l1_height` are filled with this constant so that the manifest
/// for height `h` lands at MMR index `h`. The value is non-zero because the
/// MMR encoding treats `[0; 32]` as "no peak present"; the specific bytes do
/// not affect protocol semantics, since no proof verifies against a prefilled
/// position.
// TODO(STR-3417): move to strata-identifiers
pub const MMR_SENTINEL_DUMMY_LEAF: [u8; 32] = [0xffu8; 32];
