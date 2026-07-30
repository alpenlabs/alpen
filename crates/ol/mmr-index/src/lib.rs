//! Maps OL-owned MMR state to DB-side MMR index comparisons.

mod error;
mod mmr_divergence;
mod mmr_reconcile;
#[cfg(test)]
mod test_utils;

pub use error::OLMmrIndexError;
pub use mmr_divergence::{
    DivergentOLMmrIndex, MmrIndexEntry, OLMmrIndexAhead, OLMmrIndexBehind, OLMmrIndexDivergence,
    OLMmrIndexStateMismatch, find_divergent_ol_mmr_indexes, resolve_ol_mmr_target,
};
pub use mmr_reconcile::{
    MmrIndexReconcilePlan, MmrIndexReconcileReport, MmrIndexTruncation,
    build_mmr_index_reconcile_plan,
};
