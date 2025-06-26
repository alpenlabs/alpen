//! Execution outputs.

use strata_primitives::prelude::*;
use strata_state::state_op::WriteBatch;

/// Container for the output of executing an epoch.
///
/// This is relevant for OL->ASM signalling.
#[derive(Debug, Clone)]
pub struct EpochExecutionOutput {
    /// The final state after applying the L1 check-in.
    final_state_root: Buf32,

    /// Collected logs from all of the blocks.
    logs: Vec<LogMessage>,

    /// New writes on top of the previous epoch's state.
    write_batch: WriteBatch,
}

impl EpochExecutionOutput {
    pub fn new(final_state_root: Buf32, logs: Vec<LogMessage>, write_batch: WriteBatch) -> Self {
        Self {
            final_state_root,
            logs,
            write_batch,
        }
    }

    pub fn final_state_root(&self) -> &Buf32 {
        &self.final_state_root
    }

    pub fn logs(&self) -> &[LogMessage] {
        &self.logs
    }

    pub fn write_batch(&self) -> &WriteBatch {
        &self.write_batch
    }

    pub fn add_log(&mut self, log: LogMessage) {
        self.logs.push(log);
    }

    pub fn logs_iter(&self) -> impl Iterator<Item = &LogMessage> + '_ {
        self.logs.iter()
    }
}

/// Describes the output of executing a block.
#[derive(Debug, Clone)]
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
    write_batch: WriteBatch,
}

impl BlockExecutionOutput {
    pub fn new(computed_state_root: Buf32, logs: Vec<LogMessage>, write_batch: WriteBatch) -> Self {
        Self {
            computed_state_root,
            logs,
            write_batch,
        }
    }

    pub fn computed_state_root(&self) -> &Buf32 {
        &self.computed_state_root
    }

    pub fn logs(&self) -> &[LogMessage] {
        &self.logs
    }

    pub fn write_batch(&self) -> &WriteBatch {
        &self.write_batch
    }

    pub fn add_log(&mut self, log: LogMessage) {
        self.logs.push(log);
    }

    pub fn logs_iter(&self) -> impl Iterator<Item = &LogMessage> + '_ {
        self.logs.iter()
    }
}

#[derive(Debug, Clone)]
pub struct CheckinExecutionOutput {
    computed_state_root: Buf32,
    logs: Vec<LogMessage>,
    write_batch: WriteBatch,
}

impl CheckinExecutionOutput {
    pub fn new(computed_state_root: Buf32, logs: Vec<LogMessage>, write_batch: WriteBatch) -> Self {
        Self {
            computed_state_root,
            logs,
            write_batch,
        }
    }

    pub fn computed_state_root(&self) -> Buf32 {
        self.computed_state_root
    }

    pub fn logs(&self) -> &[LogMessage] {
        &self.logs
    }

    pub fn write_batch(&self) -> &WriteBatch {
        &self.write_batch
    }
}

/// Serialized log message.
///
/// This is used for OL->ASM messaging.
///
/// Payload SHOULD conform to SPS-msg-fmt.
#[derive(Debug, Clone)]
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
