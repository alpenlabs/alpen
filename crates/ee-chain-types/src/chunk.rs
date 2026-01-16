//! Chunk types exposed to the EE account.

use strata_acct_types::Hash;

use crate::{ChunkTransition, ExecInputs, ExecOutputs};

impl ChunkTransition {
    pub fn new(
        parent_exec_blkid: Hash,
        tip_exec_blkid: Hash,
        inputs: ExecInputs,
        outputs: ExecOutputs,
    ) -> Self {
        Self {
            parent_exec_blkid: parent_exec_blkid.0.into(),
            tip_exec_blkid: tip_exec_blkid.0.into(),
            inputs,
            outputs,
        }
    }
}

// TODO whatever proptest stuff?
