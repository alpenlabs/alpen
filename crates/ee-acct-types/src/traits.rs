use crate::{
    errors::{EnvError, EnvResult},
    outputs::ExecBlockOutputs,
};

pub type Hash = [u8; 32];

/// Execution environment.
pub trait ExecutionEnvironment: Sized {
    /// Partial execution chain state.
    type PartialState<'s>: Sized;

    /// Execution block.
    type Block<'b>: Sized;

    /// Compacted version of a block that must be "executionally equivalent" to
    /// the block.
    type Summary<'b>: Sized;

    /// Write batch that can be applied to the partial state.
    type WriteBatch: Sized;

    /// Decodes a partial state from a buf.
    fn decode_partial_state<'s>(buf: &'s [u8]) -> EnvResult<Self::PartialState<'s>>;

    /// Computes the state root of a partial state.
    fn compute_state_root(ps: &Self::PartialState<'_>) -> EnvResult<Hash>;

    /// Decodes a block from a buf.
    fn decode_block<'b>(buf: &'b [u8]) -> EnvResult<Self::Block<'b>>;

    /// Decodes a block summary from a buf.
    fn decode_block_summary<'b>(buf: &'b [u8]) -> EnvResult<Self::Summary<'b>>;

    /// Computes a block's ID from the decoded block.
    fn compute_block_id(b: &Self::Block<'_>) -> EnvResult<Hash>;

    /// Processes a block, returning a block execution output.
    fn process_block(
        &self,
        pre_state: &Self::PartialState<'_>,
        block: &Self::Block<'_>,
    ) -> EnvResult<ExecBlockOutputs<Self>>;

    /// Processes a summary, returning a block execution output.
    fn process_block_summary(
        &self,
        pre_state: &Self::PartialState<'_>,
        summary: &Self::Summary<'_>,
    ) -> EnvResult<ExecBlockOutputs<Self>>;

    /// Checks if a block summary is executionally-equivalent to the block, such
    /// that it produces the same post-state and outputs.
    fn verify_block_summary(
        &self,
        pre_state: &Self::PartialState<'_>,
        block: &Self::Block<'_>,
        summary: &Self::Summary<'_>,
    ) -> EnvResult<bool>;

    /// Applies a pending write batch into the partial state.
    fn merge_write_into_state(
        &self,
        state: &Self::PartialState<'_>,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()>;
}
