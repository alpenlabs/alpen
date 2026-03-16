//! State bookkeeping necessary for ASM to run.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{de::Error as SerdeDeError, Deserialize, Serialize};
use ssz::{Decode, Encode};
use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_asm_stf::AsmStfOutput;

/// ASM bookkeping "umbrella" state.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct AsmState {
    state: AnchorState,
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

impl Serialize for AsmState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Raw<'a> {
            state: Vec<u8>,
            logs: &'a [AsmLogEntry],
        }

        Raw {
            state: self.state.as_ssz_bytes(),
            logs: &self.logs,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AsmState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            state: Vec<u8>,
            logs: Vec<AsmLogEntry>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let state = AnchorState::from_ssz_bytes(&raw.state).map_err(SerdeDeError::custom)?;
        Ok(Self {
            state,
            logs: raw.logs,
        })
    }
}
