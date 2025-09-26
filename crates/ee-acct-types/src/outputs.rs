use strata_ee_chain_types::BlockOutputs;

use crate::traits::ExecutionEnvironment;

/// Outputs produced from an block's execution.
#[derive(Debug)]
pub struct ExecBlockOutput<E: ExecutionEnvironment> {
    write_batch: E::WriteBatch,
    outputs: BlockOutputs,
    // TODO
}

impl<E: ExecutionEnvironment> ExecBlockOutput<E> {
    pub fn new(write_batch: E::WriteBatch, outputs: BlockOutputs) -> Self {
        Self {
            write_batch,
            outputs,
        }
    }

    pub fn write_batch(&self) -> &E::WriteBatch {
        &self.write_batch
    }

    pub fn outputs(&self) -> &BlockOutputs {
        &self.outputs
    }
}
