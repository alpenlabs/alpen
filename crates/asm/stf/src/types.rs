use strata_asm_common::{AnchorState, AsmLog, TxInputRef};

/// Output of applying the Anchor State Machine (ASM) state transition function
#[derive(Debug, Clone)]
pub struct AsmStfOutput {
    pub state: AnchorState,
    pub logs: Vec<AsmLog>,
}

impl AsmStfOutput {
    pub fn new(state: AnchorState, logs: Vec<AsmLog>) -> Self {
        Self { state, logs }
    }
}

/// Ouptut of preprocessing for ASM STF
#[derive(Debug)]
pub struct AsmPreProcessOutput<'t> {
    pub aux_requests: Vec<u8>,
    pub txs: Vec<TxInputRef<'t>>,
}
