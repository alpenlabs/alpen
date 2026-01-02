//! Output tracking structures.

use std::{cell::RefCell, iter};

use strata_ol_chain_types_new::OLLog;

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

    pub fn emit_logs(&self, iter: impl IntoIterator<Item = OLLog>) {
        let mut logs = self.logs.borrow_mut();
        logs.extend(iter);
    }

    pub fn into_logs(self) -> Vec<OLLog> {
        self.logs.into_inner()
    }
}

/// General trait for things that can collect exec outputs.
pub trait OutputCtx {
    /// Records some logs.
    fn emit_logs(&self, logs: impl IntoIterator<Item = OLLog>);

    /// Records a single log.
    fn emit_log(&self, log: OLLog) {
        self.emit_logs(iter::once(log));
    }
}
