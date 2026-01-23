//! Chunk data structures.

use strata_ee_acct_types::ExecutionEnvironment;
use strata_ee_chain_types::{ExecInputs, ExecOutputs};

pub struct Chunk<'c, E: ExecutionEnvironment> {
    blocks: Vec<ChunkBlock<'c, E>>,
}

impl<'c, E: ExecutionEnvironment> Chunk<'c, E> {
    pub fn new(blocks: Vec<ChunkBlock<'c, E>>) -> Self {
        Self { blocks }
    }

    pub fn blocks(&self) -> &[ChunkBlock<'c, E>] {
        &self.blocks
    }
}

pub struct ChunkBlock<'c, E: ExecutionEnvironment> {
    inputs: &'c ExecInputs,
    outputs: &'c ExecOutputs,
    exec_block: E::Block,
}

impl<'c, E: ExecutionEnvironment> ChunkBlock<'c, E> {
    pub fn new(inputs: &'c ExecInputs, outputs: &'c ExecOutputs, exec_block: E::Block) -> Self {
        Self {
            inputs,
            outputs,
            exec_block,
        }
    }

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
