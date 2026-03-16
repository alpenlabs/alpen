//! State bookkeeping necessary for ASM to run.

use std::io;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{de::Error as SerdeDeError, Deserialize, Serialize};
use ssz::{Decode, Encode};
use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_asm_stf::AsmStfOutput;

/// ASM bookkeping "umbrella" state.
#[derive(Debug, Clone, PartialEq)]
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

impl BorshSerialize for AsmState {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        let state_bytes = self.state.as_ssz_bytes();
        let log_bytes = self
            .logs
            .iter()
            .map(AsmLogEntry::as_ssz_bytes)
            .collect::<Vec<_>>();
        BorshSerialize::serialize(&(state_bytes, log_bytes), writer)
    }
}

impl BorshDeserialize for AsmState {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let (state_bytes, log_bytes): (Vec<u8>, Vec<Vec<u8>>) =
            BorshDeserialize::deserialize_reader(reader)?;
        let state = AnchorState::from_ssz_bytes(&state_bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let logs = log_bytes
            .into_iter()
            .map(|bytes| {
                AsmLogEntry::from_ssz_bytes(&bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { state, logs })
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
