use strata_codec::Codec;
use strata_ee_chain_types::BlockInputs;

use crate::{errors::EnvResult, outputs::ExecBlockOutput};

type Hash = [u8; 32];

/// Represents a partially-loaded state, including any information we would need
/// to manipulate it and compute state roots.
pub trait ExecPartialState: Codec + Sized {
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

/// Represents a full execution block, with whatever information needed to
/// execute it on top of a pre-state.
pub trait ExecBlock: Codec + Sized {
    /// The block's header type.
    type Header: ExecHeader;

    /// Gets the block's header.
    fn get_header(&self) -> Self::Header;
}

/// Execution environment.
pub trait ExecutionEnvironment: Sized {
    /// Partial execution chain state.
    type PartialState: ExecPartialState;

    /// Execution block.
    type Block: ExecBlock;

    /// Write batch that can be applied to the partial state.
    type WriteBatch: Sized;

    /// Processes a block, returning a block execution output.
    fn process_block(
        &self,
        pre_state: &Self::PartialState,
        block: &Self::Block,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<Self>>;

    /// Applies a pending write batch into the partial state.
    fn merge_write_into_state(
        &self,
        state: &mut Self::PartialState,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()>;
}
