use strata_db_types::MmrId;
use strata_identifiers::AccountId;
use strata_ledger_types::StateError;
use thiserror::Error;

/// Validation failures for an OL MMR index reconciliation plan.
#[derive(Debug, Error)]
pub enum OLMmrIndexError {
    /// The target L1 block refs MMR has no genesis sentinel.
    #[error("MMR l1-block-refs target is missing the genesis sentinel")]
    L1BlockRefsMissingSentinel,

    /// A persisted MMR index has fewer leaves than the target.
    #[error(
        "MMR {mmr_id} is behind target (index leaf count {index_leaf_count}, target leaf count {target_leaf_count})"
    )]
    BehindTarget {
        /// MMR namespace that is behind the target.
        mmr_id: MmrId,

        /// Persisted leaf count in the MMR index.
        index_leaf_count: u64,

        /// Leaf count expected by the target OL state.
        target_leaf_count: u64,
    },

    /// A persisted MMR index has target-count leaves but a diverging state.
    #[error("MMR {mmr_id} state does not match target at leaf count {leaf_count}")]
    StateMismatch {
        /// MMR namespace whose state diverges.
        mmr_id: MmrId,

        /// Leaf count where the state diverges.
        leaf_count: u64,
    },

    /// State access failed while reading a target snark inbox MMR.
    #[error("failed to read target snark account {account_id}")]
    StateAccess {
        /// Account whose target state could not be read.
        account_id: AccountId,

        /// State accessor failure.
        #[source]
        source: StateError,
    },
}
