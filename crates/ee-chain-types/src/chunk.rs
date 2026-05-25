//! Chunk types exposed to the EE account.

use strata_acct_types::Hash;

use crate::{ChunkTransition, ExecHeaderSummary, ExecInputs, ExecOutputs};

impl ExecHeaderSummary {
    pub fn new(opaque_bytes: Vec<u8>) -> Self {
        Self {
            opaque_bytes: opaque_bytes
                .try_into()
                .expect("exec header summary must fit within SSZ max length"),
        }
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

// TODO whatever proptest stuff?
