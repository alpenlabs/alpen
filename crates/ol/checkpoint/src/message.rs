//! Service input messages for OL checkpoint builder.

use strata_service::CommandCompletionSender;

use crate::errors::WorkerResult;

/// Input messages for the OL checkpoint service.
#[derive(Debug)]
pub enum OLCheckpointMessage {
    /// Triggers a single polling cycle.
    Tick(CommandCompletionSender<WorkerResult<()>>),
}
