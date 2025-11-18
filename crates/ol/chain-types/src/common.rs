use strata_identifiers::Buf32;

// Type aliases for clarity
pub type Slot = u64;
pub type Epoch = u32;

// Reexports of types that were redundant in this crate.
pub use strata_identifiers::{
    EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
};
