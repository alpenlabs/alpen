use std::collections::BTreeMap;

use bitcoin::block::Header;
use strata_asm_common::{AnchorState, AsmLogEntry, AuxPayload, AuxRequest, TxInputRef};
use strata_l1_txfmt::SubprotocolId;

/// Output of applying the Anchor State Machine (ASM) state transition function
#[derive(Debug, Clone)]
pub struct AsmStfOutput {
    pub state: AnchorState,
    pub logs: Vec<AsmLogEntry>,
}

impl AsmStfOutput {
    pub fn new(state: AnchorState, logs: Vec<AsmLogEntry>) -> Self {
        Self { state, logs }
    }
}

/// Output of preprocessing for ASM STF
#[derive(Debug)]
pub struct AsmPreProcessOutput<'t> {
    pub aux_requests: Vec<AuxRequest>,
    pub txs: Vec<TxInputRef<'t>>,
}

/// Input for ASM STF
#[derive(Debug)]
pub struct AsmStfInput<'b, 'x> {
    pub header: &'b Header,
    pub protocol_txs: BTreeMap<SubprotocolId, Vec<TxInputRef<'b>>>,
    pub aux_input: &'x BTreeMap<SubprotocolId, Vec<AuxPayload>>,
}
