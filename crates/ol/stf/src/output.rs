//! Output tracking structures.

use std::{cell::RefCell, iter};

use strata_ol_chain_types_new::{MAX_LOGS_PER_BLOCK, OLLog};

use crate::errors::{ExecError, ExecResult};

/// Collector for outputs that we can pass around between different contexts.
#[derive(Clone, Debug)]
pub struct ExecOutputBuffer {
    // maybe we'll have stuff other than logs in the future
    // TODO don't use refcell, this sucks
    logs: RefCell<Vec<OLLog>>,
}

impl ExecOutputBuffer {
    fn new(logs: Vec<OLLog>) -> Self {
        Self {
            logs: RefCell::new(logs),
        }
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn emit_logs(&self, iter: impl IntoIterator<Item = OLLog>) -> ExecResult<()> {
        let new_logs: Vec<OLLog> = iter.into_iter().collect();
        let mut logs = self.logs.borrow_mut();
        let max = MAX_LOGS_PER_BLOCK as usize;
        let total = logs.len().saturating_add(new_logs.len());
        if total > max {
            return Err(ExecError::LogsOverflow { count: total, max });
        }
        logs.extend(new_logs);
        Ok(())
    }

    pub fn snapshot_logs(&self) -> Vec<OLLog> {
        self.logs.borrow().clone()
    }

    pub fn log_count(&self) -> usize {
        self.logs.borrow().len()
    }

    pub fn verify_logs_within_block_limit(&self) -> ExecResult<()> {
        let count = self.log_count();
        let max = MAX_LOGS_PER_BLOCK as usize;
        if count > max {
            return Err(ExecError::LogsOverflow { count, max });
        }
        Ok(())
    }

    pub fn into_logs(self) -> Vec<OLLog> {
        self.logs.into_inner()
    }
}

/// General trait for things that can collect exec outputs.
pub trait OutputCtx {
    /// Records some logs. Returns an error if the block log cap would be exceeded.
    fn emit_logs(&self, logs: impl IntoIterator<Item = OLLog>) -> ExecResult<()>;

    /// Records a single log. Returns an error if the block log cap would be exceeded.
    fn emit_log(&self, log: OLLog) -> ExecResult<()> {
        self.emit_logs(iter::once(log))
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AccountSerial;
    use strata_ol_chain_types_new::{MAX_LOGS_PER_BLOCK, OLLog};

    use super::ExecOutputBuffer;
    use crate::ExecError;

    #[test]
    fn test_log_count_tracks_emitted_logs() {
        let output = ExecOutputBuffer::new_empty();
        assert_eq!(output.log_count(), 0);

        output
            .emit_logs([
                OLLog::new(AccountSerial::from(1u32), vec![1]),
                OLLog::new(AccountSerial::from(2u32), vec![2]),
            ])
            .unwrap();
        assert_eq!(output.log_count(), 2);
    }

    #[test]
    fn test_verify_logs_within_block_limit() {
        let output = ExecOutputBuffer::new_empty();
        output
            .emit_logs(
                (0..MAX_LOGS_PER_BLOCK).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])),
            )
            .unwrap();
        assert!(output.verify_logs_within_block_limit().is_ok());
    }

    #[test]
    fn test_emit_logs_rejects_above_cap() {
        let output = ExecOutputBuffer::new_empty();
        output
            .emit_logs(
                (0..MAX_LOGS_PER_BLOCK).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])),
            )
            .unwrap();

        let err = output
            .emit_logs([OLLog::new(AccountSerial::from(0u32), vec![])])
            .unwrap_err();
        assert!(matches!(
            err,
            ExecError::LogsOverflow { count, max }
                if count == (MAX_LOGS_PER_BLOCK as usize + 1)
                    && max == MAX_LOGS_PER_BLOCK as usize
        ));
        // Buffer should not have grown — emission was rejected.
        assert_eq!(output.log_count(), MAX_LOGS_PER_BLOCK as usize);
    }
}
