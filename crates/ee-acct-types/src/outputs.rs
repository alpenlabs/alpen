use strata_ee_chain_types::ExecOutputs;

use crate::traits::ExecutionEnvironment;

/// Outputs produced from an block's execution.
#[derive(Debug)]
pub struct ExecBlockOutput<E: ExecutionEnvironment> {
    write_batch: E::WriteBatch,
    block_output: E::BlockOutput,
    outputs: ExecOutputs,
}

impl<E: ExecutionEnvironment> ExecBlockOutput<E> {
    pub fn new(write_batch: E::WriteBatch, outputs: ExecOutputs) -> Self
    where
        E::BlockOutput: Default,
    {
        Self::new_with_block_output(write_batch, E::BlockOutput::default(), outputs)
    }

    pub fn new_with_block_output(
        write_batch: E::WriteBatch,
        block_output: E::BlockOutput,
        outputs: ExecOutputs,
    ) -> Self {
        Self {
            write_batch,
            block_output,
            outputs,
        }
    }

    pub fn write_batch(&self) -> &E::WriteBatch {
        &self.write_batch
    }

    pub fn block_output(&self) -> &E::BlockOutput {
        &self.block_output
    }

    pub fn outputs(&self) -> &ExecOutputs {
        &self.outputs
    }
}
