use std::collections::BTreeMap;

use bitcoin::block::Header;
use strata_asm_common::{
    AnchorState, AsmLogEntry, AuxPayload, AuxRequest, RequesterL1Index, TxInputRef,
};
use strata_l1_txfmt::SubprotocolId;

/// Auxiliary input required to run the ASM STF.
///
/// Each subprotocol is expected to emit **at most one** auxiliary payload per L1 transaction.
/// The preprocessing collector enforces this, so the caller can index the map directly by
/// `(subprotocol id, requester l1 index)` without handling duplicates.
pub type AsmAuxInput = BTreeMap<(SubprotocolId, RequesterL1Index), AuxPayload>;

/// Auxiliary request produced during ASM STF preprocessing phase.
pub(crate) type AsmAuxRequest = BTreeMap<(SubprotocolId, RequesterL1Index), AuxRequest>;

/// Overall input to ASM STF, including opaque aux inputs.
#[derive(Debug)]
pub struct AsmStfInput<'i> {
    pub header: &'i Header,
    pub protocol_txs: BTreeMap<SubprotocolId, Vec<TxInputRef<'i>>>,
    pub aux_input: &'i AsmAuxInput,
}

/// Output of ASM input preprocessing.
#[derive(Debug)]
pub struct AsmPreProcessOutput<'i> {
    pub txs: Vec<TxInputRef<'i>>,
    pub aux_requests: AsmAuxRequest,
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
