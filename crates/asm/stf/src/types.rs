use std::collections::BTreeMap;

use bitcoin::block::Header;
use strata_asm_common::{AnchorState, AsmManifest, AuxData, AuxRequests, TxInputRef};
use strata_identifiers::Buf32;
use strata_l1_txfmt::SubprotocolId;

/// Overall input to ASM STF, including opaque aux inputs.
#[derive(Debug)]
pub struct AsmStfInput<'i> {
    pub header: &'i Header,
    pub wtxids_root: Buf32,
    pub aux_data: AuxData,
    pub protocol_txs: BTreeMap<SubprotocolId, Vec<TxInputRef<'i>>>,
}

/// Output of ASM input preprocessing.
#[derive(Debug)]
pub struct AsmPreProcessOutput<'i> {
    pub txs: BTreeMap<SubprotocolId, Vec<TxInputRef<'i>>>,
    pub aux_requests: BTreeMap<SubprotocolId, AuxRequests>,
}

/// Overall output of applying ASM STF.
#[derive(Debug, Clone, PartialEq)]
pub struct AsmStfOutput {
    pub state: AnchorState,
    pub manifest: AsmManifest,
}

impl AsmStfOutput {
    pub fn new(state: AnchorState, manifest: AsmManifest) -> Self {
        Self { state, manifest }
    }
}
