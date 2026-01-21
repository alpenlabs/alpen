//! Chunk data structures.

use strata_ee_acct_types::ExecutionEnvironment;
use strata_ee_chain_types::{ExecInputs, ExecOutputs};

pub struct Chunk<'c, E: ExecutionEnvironment> {
    blocks: Vec<Chunk<'c, E>>,
}

impl<'c, E: ExecutionEnvironment> Chunk<'c, E> {
    pub fn blocks(&self) -> &[Chunk<'c, E>] {
        &self.blocks
    }
}

pub struct ChunkBlock<'c, E: ExecutionEnvironment> {
    inputs: &'c ExecInputs,
    outputs: &'c ExecOutputs,
    exec_block: E::Block,
}

impl<'c, E: ExecutionEnvironment> ChunkBlock<'c, E> {
    pub fn inputs(&self) -> &'c ExecInputs {
        self.inputs
    }

    pub fn outputs(&self) -> &'c ExecOutputs {
        self.outputs
    }

    pub fn exec_block(&self) -> &E::Block {
        &self.exec_block
    }
}
