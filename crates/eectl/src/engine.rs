// I worry that this design is broken down a bit too much and would lead to more
// RPC traffic with the EL node if we do go down and just make this be a wrapper
// to the engine API.  Perhaps we should make this a message oriented
// architecture where we produce a bundle of changes to update our expected idea
// of the EL state and then after applying a bundle of changes we can then go
// and update the external EL state.  That would mean going and making a more
// minimal set of EL calls.  This design would probably be more able to handle
// errors as we'd be able to identify when our perspective on the state is
// inconsistent with the remote state.

use strata_state::{block::L2BlockBundle, id::L2BlockId};

use crate::{errors::*, messages::*};

/// Interface to control an execution engine.  This is defined in terms of
/// strata semantics which will be produced inside the EL impl according to
/// whatever semantics it has.
pub trait ExecEngineCtl {
    /// Execute a block payload to determine its validity and if it extends the
    /// current chain tip.
    ///
    /// Corresponds to `engine_newPayloadVX`.
    fn submit_payload(&self, payload: ExecPayloadData) -> EngineResult<BlockStatus>;

    /// Tries to prepare a payload using the current state of the chain,
    /// returning an ID to query pending payload build jobs.  If this completes
    /// successfully and then `.update_head_block` is called on it, will
    /// broadcast new payload to peers.
    fn prepare_payload(&self, env: PayloadEnv) -> EngineResult<u64>;

    /// Tries to get a payload that we were working on.
    fn get_payload_status(&self, id: u64) -> EngineResult<PayloadStatus>;

    /// Updates the (L2) block that we treat as the chain tip and build new
    /// blocks on.
    fn update_head_block(&self, id: L2BlockId) -> EngineResult<()>;

    /// Updates the (L2) block that we treat as the safe chain tip that we
    /// respond to RPCs with.
    fn update_safe_block(&self, id: L2BlockId) -> EngineResult<()>;

    /// Updates the (L2) block that we treat as being deeply buried and won't
    /// reorg.
    fn update_finalized_block(&self, id: L2BlockId) -> EngineResult<()>;

    /// Check if a block exists on the chain.
    /// If this returns true, it should be safe to use this id
    /// in any of update_*_block methods, submit_payload and prepare_payload
    fn check_block_exists<'a>(&self, id_ref: L2BlockRef<'a>) -> EngineResult<bool>;
}

#[derive(Debug)]
pub enum L2BlockRef<'a> {
    Id(L2BlockId),
    Ref(&'a L2BlockBundle),
}

/// The status of a block that we've just set chain fork.
///
/// Corresponds to `Forkchoice
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BlockStatus {
    /// The block tip is valid.
    Valid,

    /// The block tip is invalid, reason doesn't matter.
    Invalid,

    /// We are still syncing previous blocks and don't have the ability to
    /// figure out this query yet.
    Syncing,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum PayloadStatus {
    /// Still building the payload.
    Working,

    /// Completed, with short commitment payload data and gas used by the block.
    Ready(ExecPayloadData, u64),
}
