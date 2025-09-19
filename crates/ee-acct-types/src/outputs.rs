use crate::traits::ExecutionEnvironment;

/// Outputs produced from an block's execution.
#[derive(Debug)]
pub struct ExecBlockOutputs<E: ExecutionEnvironment> {
    write_batch: E::WriteBatch,
    // TODO
}

impl<E: ExecutionEnvironment> ExecBlockOutputs<E> {
    pub fn new(write_batch: E::WriteBatch) -> Self {
        Self { write_batch }
    }

    pub fn write_batch(&self) -> &E::WriteBatch {
        &self.write_batch
    }
}
