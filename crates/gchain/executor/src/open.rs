//! High-level logic related to opening an executor on top of a store.

use strata_gchain_types::*;

use crate::core::*;
use crate::errors::GExecError;
use crate::linear_executor::LinearExecutor;
use crate::reprocess::{RejectedCommit, ReprocExecutor, StagePlan};
use crate::store::ExecutorStore;

/// What opening an executor did to bring the stored state in line with the
/// pipeline it was opened with.
pub struct OpenReport<S: GChainSpec> {
    initialized: Vec<ProcId>,
    recommitted: Vec<ProcId>,
    reprocessed: Vec<StagePlan<S>>,
    rejected: Vec<RejectedCommit<S>>,
}

impl<S: GChainSpec> OpenReport<S> {
    /// Stages that had no committed state and were initialized at the
    /// committed node.
    pub fn initialized(&self) -> &[ProcId] {
        &self.initialized
    }

    /// Stages whose committed node lagged the pipeline's and had the rest of
    /// the committed path committed again.
    pub fn recommitted(&self) -> &[ProcId] {
        &self.recommitted
    }

    /// The committed links each stage processed again, because it no longer
    /// accepted the artifacts stored for them.
    pub fn reprocessed(&self) -> &[StagePlan<S>] {
        &self.reprocessed
    }

    /// Committed links a stage rejected when processing them again, which
    /// the pipeline was rolled back to before.
    pub fn rejected(&self) -> &[RejectedCommit<S>] {
        &self.rejected
    }
}

/// A [`LinearExecutor`] and the output produced from readying it.
pub(crate) type LinearExecutorReadyOutput<S, P, X> = (LinearExecutor<S, P, X>, OpenReport<S>);

/// What opening an executor over a store came to.
pub enum OpenResult<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>> {
    /// The stages stand behind everything that's committed.
    Ready(LinearExecutor<S, P, X>, OpenReport<S>),

    /// Some stages have to reprocessed because some links due to a client
    /// upgrade or crash.
    NeedsReprocess(ReprocExecutor<S, P, X>),
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>> OpenResult<S, P, X> {
    /// Does whatever reprocessing is needed right away and hands over the
    /// executor.
    pub fn finish(self) -> Result<LinearExecutorReadyOutput<S, P, X>, GExecError> {
        match self {
            Self::Ready(exec, report) => Ok((exec, report)),
            Self::NeedsReprocess(pending) => pending.finish(),
        }
    }

    /// Unwraps the `Ready` case if present.  This is a convenience fn meant for
    /// tests.
    ///
    /// # Panics
    ///
    /// If not ready, meaning there's reprocessing needed.
    pub fn expect_ready(self) -> LinearExecutorReadyOutput<S, P, X> {
        match self {
            OpenResult::Ready(exec, report) => (exec, report),
            OpenResult::NeedsReprocess(_) => panic!("gchain/open: not ready"),
        }
    }
}

/// Brings the stages level with the committed node, which is all that's left
/// to do once no committed link needs reprocessing, and hands over the
/// executor with the account of what opening it took.
pub(crate) fn finish_open<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>>(
    mut core: ExecutorCore<S, P, X>,
    reprocessed: Vec<StagePlan<S>>,
    rejected: Vec<RejectedCommit<S>>,
) -> Result<LinearExecutorReadyOutput<S, P, X>, GExecError> {
    let leveled = core.level_stages()?;
    let report = OpenReport {
        initialized: leveled.initialized,
        recommitted: leveled.recommitted,
        reprocessed,
        rejected,
    };
    Ok((LinearExecutor::from_clean_core(core), report))
}
