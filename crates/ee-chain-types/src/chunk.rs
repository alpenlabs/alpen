//! Chunk types exposed to the EE account.

use strata_acct_types::Hash;

use crate::{ChunkTransition, ExecInputs, ExecOutputs};

impl ChunkTransition {
    pub fn new(
        parent_exec_blkid: Hash,
        tip_exec_blkid: Hash,
        tip_state_root: Hash,
        inputs: ExecInputs,
        outputs: ExecOutputs,
    ) -> Self {
        Self {
            parent_exec_blkid: parent_exec_blkid.0.into(),
            tip_exec_blkid: tip_exec_blkid.0.into(),
            tip_state_root: tip_state_root.0.into(),
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

    /// Post-execution state root of the chunk's tip block.
    pub fn tip_state_root(&self) -> Hash {
        self.tip_state_root.0.into()
    }

    pub fn inputs(&self) -> &ExecInputs {
        &self.inputs
    }

    pub fn outputs(&self) -> &ExecOutputs {
        &self.outputs
    }
}

// TODO whatever proptest stuff?
