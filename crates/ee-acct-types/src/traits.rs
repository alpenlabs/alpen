use strata_codec::Codec;
use strata_ee_chain_types::ExecInputs;
use strata_identifiers::Hash;

use crate::{errors::EnvResult, inputs::ExecPayload, outputs::ExecBlockOutput};

/// Represents a partially-loaded state, including any information we would need
/// to manipulate it and compute state roots.
pub trait ExecPartialState: Codec + Clone + Sized {
    /// Computes the state root of a partial state.
    fn compute_state_root(&self) -> EnvResult<Hash>;
}

/// Represents an execution block header.
pub trait ExecHeader: Codec + Clone + Sized {
    /// Data intrinsic to a block header, like the timestamp and parent block.
    ///
    /// This doesn't include data that's a commitment to external data, like
    /// the body or anything that's computed as a result of execution.
    ///
    /// In practice, this MAY be the same structure as a header, but with some
    /// fields stubbed out with dummy/zero values.
    type Intrinsics: Clone;

    /// Gets the header's intrinsics we can execute the block with.
    fn get_intrinsics(&self) -> Self::Intrinsics;

    /// Gets the state root field.
    fn get_state_root(&self) -> Hash;

    /// Computes the exec block ID.
    fn compute_block_id(&self) -> Hash;
}

/// Represents the body of an execution block, without the header.
///
/// This is the executable content of a block (transactions, operations, etc.)
/// that can be processed to produce state changes.
pub trait ExecBlockBody: Codec + Sized {}

/// Represents a full execution block, with whatever information needed to
/// execute it on top of a pre-state.
pub trait ExecBlock: Codec + Sized {
    /// The block's header type.
    type Header: ExecHeader;

    /// The block's body type.
    type Body: ExecBlockBody;

    /// Constructs a block from a header and body.
    fn from_parts(header: Self::Header, body: Self::Body) -> Self;

    /// Checks if a header matches a body.
    fn check_header_matches_body(header: &Self::Header, body: &Self::Body) -> bool;

    /// Gets a reference to the block's header.
    fn get_header(&self) -> &Self::Header;

    /// Gets a reference to the block's body.
    fn get_body(&self) -> &Self::Body;
}

/// Execution environment.
pub trait ExecutionEnvironment: Sized {
    /// Partial execution chain state.
    type PartialState: ExecPartialState;

    /// Execution block.
    type Block: ExecBlock;

    /// Write batch that can be applied to the partial state.
    ///
    /// This is NOT any kind of writes we check for in DA.
    type WriteBatch: Sized;

    /// Executes a block payload on top of a pre-state, returning the execution
    /// output.
    ///
    /// This should still be checked against a header when verifying a block, or
    /// can be used to construct the final block header when assembling a block.
    fn execute_block_body(
        &self,
        pre_state: &Self::PartialState,
        exec_payload: &ExecPayload<'_, Self::Block>,
        inputs: &ExecInputs,
    ) -> EnvResult<ExecBlockOutput<Self>>;

    /// Performs any additional checks needed from the block outputs against the
    /// header.
    fn verify_outputs_against_header(
        &self,
        header: &<Self::Block as ExecBlock>::Header,
        outputs: &ExecBlockOutput<Self>,
    ) -> EnvResult<()>;

    /// Applies a pending write batch into the partial state.
    // NOTE: should this be moved to BlockAssembler?
    fn merge_write_into_state(
        &self,
        state: &mut Self::PartialState,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()>;
}

/// Block assembly trait for constructing complete headers from execution outputs.
///
/// This trait is separate from ExecutionEnvironment because header completion
/// is only needed when building new blocks, not when verifying existing blocks.
/// The ExecutionEnvironment trait focuses on proof-related operations.
pub trait BlockAssembler: ExecutionEnvironment {
    /// Constructs a complete header from header intrinsics and execution outputs.
    ///
    /// This combines the header intrinsics (parent hash, timestamp, etc.) with
    /// the computed commitments from execution (state root, logs bloom, etc.)
    /// to produce a complete block header.
    fn complete_header(
        &self,
        exec_payload: &ExecPayload<'_, Self::Block>,
        output: &ExecBlockOutput<Self>,
    ) -> EnvResult<<Self::Block as ExecBlock>::Header>;
}
