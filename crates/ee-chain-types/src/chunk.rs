//! Chunk types exposed to the EE account.

use ssz_types::{Error as SszError, VariableList};
use strata_acct_types::Hash;

use crate::{ChunkTransition, ExecHeaderSummary, ExecInputs, ExecOutputs};

impl ExecHeaderSummary {
    /// Creates a summary from an already-bounded opaque byte list.
    pub fn new(opaque_bytes: VariableList<u8, 1024>) -> Self {
        Self { opaque_bytes }
    }

    /// Creates a summary from raw bytes, returning an error if they exceed the
    /// SSZ bound (`MAX_EXEC_HEADER_SUMMARY_BYTES` = 1024).
    pub fn from_vec(opaque_bytes: Vec<u8>) -> Result<Self, SszError> {
        Ok(Self {
            opaque_bytes: opaque_bytes.try_into()?,
        })
    }

    /// Creates an empty summary.
    pub fn new_empty() -> Self {
        Self::from_vec(Vec::new()).expect("empty opaque bytes fit the SSZ bound")
    }

    pub fn opaque_bytes(&self) -> &[u8] {
        &self.opaque_bytes
    }
}

impl ChunkTransition {
    pub fn new(
        parent_exec_blkid: Hash,
        tip_exec_blkid: Hash,
        tip_state_root: Hash,
        tip_exec_header_summary: ExecHeaderSummary,
        inputs: ExecInputs,
        outputs: ExecOutputs,
    ) -> Self {
        Self {
            parent_exec_blkid: parent_exec_blkid.0.into(),
            tip_exec_blkid: tip_exec_blkid.0.into(),
            tip_state_root: tip_state_root.0.into(),
            tip_exec_header_summary,
            inputs,
            outputs,
        }
    }

    pub fn parent_exec_blkid(&self) -> Hash {
        self.parent_exec_blkid.0.into()
    }

    pub fn tip_exec_blkid(&self) -> Hash {
        self.tip_exec_blkid.0.into()
    }

    pub fn tip_state_root(&self) -> Hash {
        self.tip_state_root.0.into()
    }

    pub fn tip_exec_header_summary(&self) -> &ExecHeaderSummary {
        &self.tip_exec_header_summary
    }

    pub fn inputs(&self) -> &ExecInputs {
        &self.inputs
    }

    pub fn outputs(&self) -> &ExecOutputs {
        &self.outputs
    }
}

// TODO(STR-3685): whatever proptest stuff?
