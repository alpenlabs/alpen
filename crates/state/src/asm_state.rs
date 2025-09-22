//! State bookkeeping necessary for ASM to run.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_asm_stf::AsmStfOutput;

/// ASM bookkeping "umbrella" state.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmState {
    state: AnchorState,
    #[serde(with = "asm_log_entry_vec_hex")]
    logs: Vec<AsmLogEntry>,
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

    pub fn logs(&self) -> &Vec<AsmLogEntry> {
        &self.logs
    }

    pub fn state(&self) -> &AnchorState {
        &self.state
    }
}

/// Serde serialization of logs as hex strings.
mod asm_log_entry_vec_hex {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::AsmLogEntry;

    pub(super) fn serialize<S>(entries: &[AsmLogEntry], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Turn each entry into a hex string of its raw bytes
        let hex_strings: Vec<String> = entries.iter().map(|e| hex::encode(&e.0)).collect();
        hex_strings.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<AsmLogEntry>, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Expect a Vec<String> of hex values
        let hex_strings: Vec<String> = Vec::<String>::deserialize(deserializer)?;
        let mut entries = Vec::with_capacity(hex_strings.len());
        for s in hex_strings {
            let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
            entries.push(AsmLogEntry(bytes));
        }
        Ok(entries)
    }
}
