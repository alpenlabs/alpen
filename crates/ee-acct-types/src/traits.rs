use strata_ee_chain_types::BlockInputs;

use crate::{errors::EnvResult, outputs::ExecBlockOutput};

type Hash = [u8; 32];

/// Execution environment.
pub trait ExecutionEnvironment: Sized {
    /// Partial execution chain state.
    type PartialState<'s>: Sized;

    /// Execution block header.
    type Header<'h>: Sized;

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

    /// Decodes a header from a buf.
    fn decode_header<'h>(buf: &'h [u8]) -> EnvResult<Self::Header<'h>>;

    /// Gets the state root from a parsed header.
    fn get_header_state_root(h: &Self::Header<'_>) -> Hash;

    /// Decodes a block from a buf.
    fn decode_block<'b>(buf: &'b [u8]) -> EnvResult<Self::Block<'b>>;

    /// Gets a block's header.
    fn get_block_header<'b>(block: &Self::Block<'b>) -> Self::Header<'b>;

    /// Decodes a block summary from a buf.
    fn decode_block_summary<'b>(buf: &'b [u8]) -> EnvResult<Self::Summary<'b>>;

    /// Computes a block's ID from its header.
    fn compute_block_id(h: &Self::Header<'_>) -> Hash;

    /// Processes a block, returning a block execution output.
    fn process_block(
        &self,
        pre_state: &Self::PartialState<'_>,
        block: &Self::Block<'_>,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<Self>>;

    /// Processes a summary, returning a block execution output.
    fn process_block_summary(
        &self,
        pre_state: &Self::PartialState<'_>,
        summary: &Self::Summary<'_>,
    ) -> EnvResult<ExecBlockOutput<Self>>;

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
        state: &mut Self::PartialState<'_>,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()>;
}
