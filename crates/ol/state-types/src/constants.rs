//! Protocol constants.

use strata_identifiers::Hash;
use strata_merkle::L1_HEIGHT_MMR_PREFILL_LEAF;

/// Just [`L1_HEIGHT_MMR_PREFILL_LEAF`] as a [`tyalias@Hash`].
pub const MMR_SENTINEL_DUMMY_LEAF_HASH: Hash = Hash::new(L1_HEIGHT_MMR_PREFILL_LEAF);
