//! CSM worker service state.

use std::sync::Arc;

use strata_asm_proto_checkpoint::{state::CheckpointState, subprotocol::CheckpointSubprotocol};
use strata_asm_txs_checkpoint::CHECKPOINT_SUBPROTOCOL_ID;
use strata_csm_types::ClientState;
use strata_identifiers::Epoch;
use strata_params::Params;
use strata_primitives::prelude::*;
use strata_service::ServiceState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

use crate::constants;

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
    pub(crate) last_processed_epoch: Option<Epoch>,

    /// Checkpoint Subprotocol State after processing last_asm_block
    pub(crate) last_checkpoint_state: Option<CheckpointState>,

    /// Status channel for publishing state updates.
    pub(crate) status_channel: Arc<StatusChannel>,
}

impl CsmWorkerState {
    /// Create a new CSM worker state.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> anyhow::Result<Self> {
        // Load the most recent client state from storage
        let (cur_block, cur_state) = storage
            .client_state()
            .fetch_most_recent_state()?
            .unwrap_or((params.rollup.genesis_l1_view.blk, ClientState::default()));

        // This can be `None` before the checkpoint subprotocol is first loaded,
        // or if no ASM state exists yet for this block.
        let checkpoint_state = storage.asm().get_state(cur_block)?.and_then(|asm_state| {
            asm_state
                .state()
                .find_section(CHECKPOINT_SUBPROTOCOL_ID)
                .map(|section| {
                    section
                        .try_to_state::<CheckpointSubprotocol>()
                        .expect("SectionState to SubprotocolState must be infallible")
                })
        });

        Ok(Self {
            _params: params,
            storage,
            cur_state: Arc::new(cur_state),
            last_asm_block: Some(cur_block),
            last_processed_epoch: None,
            last_checkpoint_state: checkpoint_state,
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
        constants::SERVICE_NAME
    }
}
