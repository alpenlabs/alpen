//! Execution outputs.

use strata_primitives::prelude::*;

use crate::ChangedState;

/// Container for the output of executing an epoch.
///
/// This is relevant for OL->ASM signalling.
pub struct EpochExecutionOutput {
    /// The final state after applying the L1 check-in.
    final_state: Buf32,

    /// Collected logs from all of the blocks.
    logs: Vec<LogMessage>,

    /// New state on top of the previous epoch's state.
    state: ChangedState,
}

/// Describes the output of executing a block.
pub struct BlockExecutionOutput {
    /// State root as computed by the STF.
    computed_state_root: Buf32,

    /// Log messages emitted while executing the block.
    ///
    /// These will eventually be accumulated to be processed by ASM.
    logs: Vec<LogMessage>,

    /// Changes to the state we store in the database.
    ///
    /// This is NOT a state diff, that requires more precise tracking.
    changes: ChangedState,
}

impl BlockExecutionOutput {
    pub fn new(computed_state_root: Buf32, logs: Vec<LogMessage>, changes: ChangedState) -> Self {
        Self {
            computed_state_root,
            logs,
            changes,
        }
    }

    pub fn computed_state_root(&self) -> &Buf32 {
        &self.computed_state_root
    }

    pub fn logs(&self) -> &[LogMessage] {
        &self.logs
    }

    pub fn changes(&self) -> &ChangedState {
        &self.changes
    }

    pub fn add_log(&mut self, log: LogMessage) {
        self.logs.push(log);
    }

    pub fn logs_iter(&self) -> impl Iterator<Item = &LogMessage> + '_ {
        self.logs.iter()
    }
}

/// Serialized log message.
///
/// This is used for OL->ASM messaging.
///
/// Payload SHOULD conform to SPS-msg-fmt.
pub struct LogMessage {
    payload: Vec<u8>,
}

impl LogMessage {
    pub fn new(payload: Vec<u8>) -> Self {
        Self { payload }
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

impl<T: AsRef<[u8]>> From<T> for LogMessage {
    fn from(value: T) -> Self {
        Self {
            payload: value.as_ref().to_vec(),
        }
    }
}
