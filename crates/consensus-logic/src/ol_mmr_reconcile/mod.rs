//! Reconciles OL-owned MMR index entries against persisted OL state.

mod context;
pub mod error;
mod reconcile;
mod target;

#[cfg(test)]
mod tests;

pub use context::OLMmrReconcileCtx;
pub use error::{OLMmrReconcileError, OLMmrReconcileResult};
pub use reconcile::reconcile_ol_mmr_index_to_target;
pub use strata_ol_mmr_index::MmrIndexReconcileReport;
pub use target::OLMmrReconcileTarget;
