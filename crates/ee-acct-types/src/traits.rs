use strata_codec::Codec;
use strata_ee_chain_types::BlockInputs;

use crate::{errors::EnvResult, outputs::ExecBlockOutput};

type Hash = [u8; 32];

/// Represents a partially-loaded state, including any information we would need
/// to manipulate it and compute state roots.
pub trait ExecPartialState: Codec + Clone + Sized {
    /// Computes the state root of a partial state.
    fn compute_state_root(&self) -> EnvResult<Hash>;
}

/// Represents an execution block header.
pub trait ExecHeader: Clone + Codec + Sized {
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
    type WriteBatch: Sized;

    /// Executes a block body (without header) on top of a pre-state, returning
    /// the execution output.
    ///
    /// This is used during both block assembly/construction and verification.
    /// The write batch and outputs can be used to compute the final state root.
    fn execute_block_body(
        &self,
        pre_state: &Self::PartialState,
        body: &<Self::Block as ExecBlock>::Body,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<Self>>;

    /// Applies a pending write batch into the partial state.
    fn merge_write_into_state(
        &self,
        state: &mut Self::PartialState,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()>;
}

/// Used for final assembly of a block header.
pub trait EeHeaderBuilder<E: ExecutionEnvironment> {
    /// Any block intrinsic data.
    type Intrinsics;

    /// Finalizes data from an executed block into a header.
    fn finalize_header(
        &self,
        intrin: &Self::Intrinsics,
        prev_header: &<E::Block as ExecBlock>::Header,
        body: &<E::Block as ExecBlock>::Body,
        exec_output: &ExecBlockOutput<E>,
    ) -> EnvResult<<E::Block as ExecBlock>::Header>;
}
