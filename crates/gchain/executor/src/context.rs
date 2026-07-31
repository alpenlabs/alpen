//! Context provider for infra.

use strata_gchain_types::ProcId;

/// Context trait for a chain executor.  This should only be used by one
/// executor at a time.
pub trait ExecutorContext {
    fn store_processor_output(&self, id: ProcId) -> anyhow::Result<()>;
}
