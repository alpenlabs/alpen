use std::collections::BTreeMap;

use bitcoin::block::Header;
use strata_asm_common::{AnchorState, AsmLogEntry, AuxPayload, AuxRequest, TxInputRef};
use strata_l1_txfmt::SubprotocolId;

/// Overall input to ASM STF, including opaque aux inputs.
#[derive(Debug)]
pub struct AsmStfInput<'i> {
    pub header: &'i Header,
    pub protocol_txs: BTreeMap<SubprotocolId, Vec<TxInputRef<'i>>>,
    pub aux_input: &'i BTreeMap<SubprotocolId, AuxPayload>,
}

/// Output of ASM input preprocessing.
#[derive(Debug)]
pub struct AsmPreProcessOutput<'i> {
    pub txs: Vec<TxInputRef<'i>>,
    pub aux_requests: BTreeMap<SubprotocolId, AuxRequest>,
}

/// Overall output of applying ASM STF.
#[derive(Debug, Clone, PartialEq)]
pub struct AsmStfOutput {
    pub state: AnchorState,
    pub logs: Vec<AsmLogEntry>,
}

impl AsmStfOutput {
    pub fn new(state: AnchorState, logs: Vec<AsmLogEntry>) -> Self {
        Self { state, logs }
    }
}
