//! Output tracking structures.

use std::cell::RefCell;

use strata_acct_types::AccountId;
use strata_asm_common::AsmManifest;
use strata_ol_chain_types_new::OLLog;
use strata_ol_state_types::ExecutionAuxiliaryData;
use strata_snark_acct_types::MessageEntry;

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
        self.emit_logs(std::iter::once(log));
    }
}

/// Trait for accumulating auxiliary data during execution.
/// For proving and any other context where accumulation is not needed, the methods will be noop.
pub trait AuxAccumulationCtx {
    /// Appends asm manifest.
    fn append_asm_manifest(&self, mf: &AsmManifest);

    /// Appends message for an account.
    fn append_account_message(&self, acc: &AccountId, msg: &MessageEntry);
}

/// Accumulator that actually tracks auxiliary data for DB indexing.
#[derive(Debug, Default)]
pub struct ExecAuxAccumulator {
    data: RefCell<ExecutionAuxiliaryData>,
}

impl ExecAuxAccumulator {
    pub fn new() -> Self {
        Self {
            data: RefCell::new(ExecutionAuxiliaryData::default()),
        }
    }

    pub fn finalize(self) -> ExecutionAuxiliaryData {
        self.data.into_inner()
    }
}

impl AuxAccumulationCtx for ExecAuxAccumulator {
    fn append_asm_manifest(&self, mf: &AsmManifest) {
        self.data.borrow_mut().asm_manifests.push(mf.clone());
    }

    fn append_account_message(&self, acc: &AccountId, msg: &MessageEntry) {
        self.data
            .borrow_mut()
            .account_message_additions
            .entry(*acc)
            .or_default()
            .push(msg.clone());
    }
}

/// Noop accumulator for contexts like prover where aux data is not needed.
#[derive(Debug)]
pub struct NoopAuxAccumulator;

impl AuxAccumulationCtx for NoopAuxAccumulator {
    fn append_asm_manifest(&self, _mf: &AsmManifest) {}

    fn append_account_message(&self, _acc: &AccountId, _msg: &MessageEntry) {}
}
