mod append_list;
mod hashmap;
mod list;
mod num;
mod register;

pub mod diff {
    pub use super::{
        append_list::AppendOnlyListDiff, hashmap::HashMapDiff, list::ListDiff, num::NumDiff,
        register::RegisterDiff,
    };
}

/// Re-export the diffable derive macro.
pub use strata_diff_derive::DaDiff;

pub trait Diff: Sized + Default {
    type Target;

    fn is_default(&self) -> bool;

    // TODO: maybe apply should consume self?
    fn apply(&self, source: &mut Self::Target) -> Result<(), ApplyError>;
}

/// Should be implemented by all Da diffs.
#[derive(Debug, Clone)]
pub struct DaSerializeError;

pub trait DaSerializable: Sized {
    fn serialize(&self, buf: &mut [u8]) -> Result<(), DaSerializeError>;
    fn deserialize(data: &[u8]) -> Self;
}

#[derive(Debug, Clone)]
pub struct ApplyError;
