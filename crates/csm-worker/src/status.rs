//! CSM worker service status.

use serde::Serialize;
use strata_primitives::l1::L1BlockCommitment;

/// Status information for the CSM worker service.
///
/// The CSM worker acts as a listener to ASM worker status updates, processing
/// checkpoint logs emitted by the checkpoint-v0 subprotocol.
#[derive(Clone, Debug, Serialize)]
pub struct CsmWorkerStatus {
    pub cur_block: Option<L1BlockCommitment>,
    pub last_processed_epoch: Option<u64>,
}
