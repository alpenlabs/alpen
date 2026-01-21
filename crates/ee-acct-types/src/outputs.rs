use strata_ee_chain_types::ExecOutputs;

use crate::traits::ExecutionEnvironment;

/// Outputs produced from an block's execution.
#[derive(Debug)]
pub struct ExecBlockOutput<E: ExecutionEnvironment> {
    write_batch: E::WriteBatch,
    outputs: ExecOutputs,
    // TODO
}

impl<E: ExecutionEnvironment> ExecBlockOutput<E> {
    pub fn new(write_batch: E::WriteBatch, outputs: ExecOutputs) -> Self {
        Self {
            write_batch,
            outputs,
        }
    }

    pub fn write_batch(&self) -> &E::WriteBatch {
        &self.write_batch
    }

    pub fn outputs(&self) -> &ExecOutputs {
        &self.outputs
    }
}
