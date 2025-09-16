//! State bookkeeping necessary for ASM to run.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_asm_stf::AsmStfOutput;
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub struct AsmState {
    pub state: AnchorState,
    pub logs: Vec<AsmLogEntry>,
}

impl AsmState {
    pub fn new(state: AnchorState, logs: Vec<AsmLogEntry>) -> Self {
        Self { state, logs }
    }

    pub fn from_output(output: AsmStfOutput) -> Self {
        Self {
            state: output.state,
            logs: output.logs,
        }
    }
}
