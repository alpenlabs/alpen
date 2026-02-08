//! State bookkeeping necessary for ASM to run.

use serde::{Deserialize, Serialize};
use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_asm_stf::AsmStfOutput;
use strata_codec_utils::SszAsBytes;

/// ASM bookkeping "umbrella" state.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct AsmState {
    state: AnchorState,
    #[rkyv(with = SszAsBytes)]
    logs: Vec<AsmLogEntry>,
}

impl AsmState {
    pub fn new(state: AnchorState, logs: Vec<AsmLogEntry>) -> Self {
        Self { state, logs }
    }

    pub fn from_output(output: AsmStfOutput) -> Self {
        Self {
            state: output.state,
            logs: output.manifest.logs.to_vec(),
        }
    }

    pub fn logs(&self) -> &Vec<AsmLogEntry> {
        &self.logs
    }

    pub fn state(&self) -> &AnchorState {
        &self.state
    }
}
