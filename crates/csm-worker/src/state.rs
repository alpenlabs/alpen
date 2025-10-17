//! CSM worker service state.

use std::sync::Arc;

use strata_csm_types::ClientState;
use strata_params::Params;
use strata_primitives::prelude::*;
use strata_service::ServiceState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

/// State for the CSM worker service.
///
/// This state is used by the CSM worker which acts as a listener to ASM worker
/// status updates, processing checkpoint logs from the checkpoint-v0 subprotocol.
#[expect(
    missing_debug_implementations,
    reason = "NodeStorage doesn't implement Debug"
)]
pub struct CsmWorkerState {
    /// Consensus parameters.
    pub(crate) _params: Arc<Params>,

    /// Node storage handle.
    pub(crate) storage: Arc<NodeStorage>,

    /// Current client state.
    pub(crate) cur_state: Arc<ClientState>,

    /// Last ASM update we processed.
    pub(crate) last_asm_block: Option<L1BlockCommitment>,

    /// Last epoch we processed a checkpoint for.
    pub(crate) last_processed_epoch: Option<u64>,

    /// Status channel for publishing state updates.
    pub(crate) status_channel: StatusChannel,
}

impl CsmWorkerState {
    /// Create a new CSM worker state.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        status_channel: StatusChannel,
    ) -> anyhow::Result<Self> {
        // Load the most recent client state from storage
        let (cur_block, cur_state) = storage
            .client_state()
            .fetch_most_recent_state()?
            .expect("missing initial client state?");

        Ok(Self {
            _params: params,
            storage,
            cur_state: Arc::new(cur_state),
            last_asm_block: Some(cur_block),
            last_processed_epoch: None,
            status_channel,
        })
    }

    /// Get the last ASM block that was processed.
    pub fn last_asm_block(&self) -> Option<L1BlockCommitment> {
        self.last_asm_block
    }
}

impl ServiceState for CsmWorkerState {
    fn name(&self) -> &str {
        "csm_worker"
    }
}
