use strata_ol_chain_types_new::OLLog;
use strata_primitives::Buf32;

/// Output of a block execution
#[derive(Clone, Debug)]
pub struct ExecOutput {
    /// The resulting OL state root.
    state_root: Buf32,

    /// The resulting OL logs.
    logs: Vec<OLLog>,
    // TODO: write batch, but it will probably be handled by StateAccessor impl
}

impl ExecOutput {
    pub fn new(state_root: Buf32, logs: Vec<OLLog>) -> Self {
        Self { state_root, logs }
    }

    pub fn state_root(&self) -> Buf32 {
        self.state_root
    }

    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }
}
