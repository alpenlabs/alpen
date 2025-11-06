use std::cell::RefCell;

use strata_ol_chain_types_new::{OLBlockHeader, OLLog};
use strata_params::RollupParams;

/// Context carried throughout block execution for accumulating logs.
///
/// Uses interior mutability (`RefCell`) to allow log emission through shared references,
/// since context is passed as `&BlockExecContext` but needs to accumulate logs.
#[derive(Clone, Debug)]
pub struct BlockExecContext {
    prev_header: OLBlockHeader,
    params: RollupParams,
    logs: RefCell<Vec<OLLog>>,
}

impl BlockExecContext {
    pub fn new(prev_header: OLBlockHeader, params: RollupParams) -> Self {
        Self {
            prev_header,
            params,
            logs: RefCell::new(Vec::new()),
        }
    }

    pub fn new_with_capacity(
        prev_header: OLBlockHeader,
        params: RollupParams,
        capacity: usize,
    ) -> Self {
        Self {
            prev_header,
            params,
            logs: RefCell::new(Vec::with_capacity(capacity)),
        }
    }

    pub fn prev_header(&self) -> &OLBlockHeader {
        &self.prev_header
    }

    pub fn params(&self) -> &RollupParams {
        &self.params
    }

    pub fn into_logs(self) -> Vec<OLLog> {
        self.logs.into_inner()
    }

    /// Emits a log entry.
    ///
    /// # Panics
    /// Panics if logs are already borrowed mutably (should not occur in normal execution).
    pub fn emit_log(&self, log: OLLog) {
        self.logs.borrow_mut().push(log)
    }

    /// Emits multiple log entries.
    ///
    /// # Panics
    /// Panics if logs are already borrowed mutably (should not occur in normal execution).
    pub fn emit_logs(&self, logs: Vec<OLLog>) {
        self.logs.borrow_mut().extend(logs)
    }
}
