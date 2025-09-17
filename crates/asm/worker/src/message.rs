//! Messages from the handle to the worker.

use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;

/// Messages from the ASM Handle to the subprotocol to give it work to do.
#[derive(Debug)]
pub enum SubprotocolMessage {
    NewAsmState(AsmState, L1BlockCommitment),
}
