use std::{collections::BTreeSet, sync::Arc};

use strata_identifiers::{Epoch, OLBlockCommitment};
use strata_ol_state_types::OLState;

/// Reconciliation target for OL-owned MMR indexes and related indexing rows.
#[derive(Clone, Debug)]
pub struct OLMmrReconcileTarget {
    /// Target block commitment.
    pub block: OLBlockCommitment,

    /// Epoch that owns [`Self::block`].
    pub epoch: Epoch,

    /// Target OL state that the MMR index must match.
    pub state: Arc<OLState>,

    /// Blocks in [`Self::epoch`] whose indexing this target rejects.
    pub rejected_indexing_blocks: BTreeSet<OLBlockCommitment>,
}

impl OLMmrReconcileTarget {
    /// Creates a reconciliation target.
    pub fn new(
        block: OLBlockCommitment,
        epoch: Epoch,
        state: Arc<OLState>,
        rejected_indexing_blocks: BTreeSet<OLBlockCommitment>,
    ) -> Self {
        Self {
            block,
            epoch,
            state,
            rejected_indexing_blocks,
        }
    }
}
