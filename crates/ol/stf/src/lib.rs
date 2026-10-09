//! Versioned entry point to the OL state transition function.
//!
//! Every OL STF driver calls the STF through this crate, passing the
//! [`OLSpecId`] whose rules govern the work. The drivers are block
//! verification, sequencer block assembly and its mempool, the checkpoint
//! proof, checkpoint DA replay, epoch DA computation and resource rebuild, and
//! genesis.
//!
//! Each rule set implements one internal trait, and a single dispatch macro
//! maps spec identifiers to rule sets. A spec that reuses earlier rules adds
//! one arm to that mapping. A spec with new rules adds a trait implementation,
//! which the compiler requires to provide every operation. Sequencer block
//! assembly composes the [`sequencer`] phases itself, so a rule set that changes
//! their order or composition must also update block assembly.
//!
//! # Spec selection
//!
//! The caller chooses the spec from the epoch being executed: a block runs
//! under its header's epoch, and an epoch's terminal block, including the
//! drain that advances state to the next epoch, runs under the spec of the
//! epoch it ends. Genesis runs under the network's genesis spec,
//! [`OLParams::genesis_spec`](strata_ol_params::OLParams::genesis_spec).
//! Every later epoch runs the spec [`select_next_epoch_spec`] selects from the
//! epoch before it: the successor of that epoch's spec if that epoch processed
//! a checkpoint predicate enactment itself, its spec otherwise. The switch is
//! read from the L1 manifests every node stores, so no rule set stages or
//! promotes a spec, and the rule is the same from V0 to V1 and from each later
//! spec to the next. Checkpoint sync and the sequencer's boot checks select
//! with it.
//!
//! # Unknown and unimplemented specs
//!
//! Decoding rejects unknown identifiers, and dispatch matches every variant,
//! so a node never runs an epoch under older rules because it lacks newer
//! ones. When an enactment activates a spec this binary does not know,
//! selection returns [`UpgradeRequired`], and a driver that selects with it
//! runs nothing for the new epoch.
//!
//! [`OLSpecId::V0`] names the 0.3.0 rules, which networks launched on that
//! release run from genesis. This binary implements V0 only for building
//! such a network's genesis block and replaying its V0 epochs from checkpoint
//! DA. Every other operation under V0 returns [`ExecError::UnimplementedSpec`]
//! instead of running V0 epochs under V1 rules.
//!
//! # Root form
//!
//! The current spec selects the form of the state root: the bare chainstate
//! root under V0, which is what 0.3.0 headers commit to, and
//! `hash_tree_root(OLRootState)` from V1 on. V0 rules keep a V0 state in the
//! bare form. V1 rules take over a V0 state at the first block of an epoch,
//! whose epoch-initial processing wraps it as `cur = staged = V1`. Every
//! driver runs that processing, DA replay included, so they all reach the same
//! wrapped root. A V1 block that does not start an epoch fails on a V0 state
//! with [`ExecError::ContinuesV0Epoch`].
//!
//! # V1-compatible surface
//!
//! The context, output, and error types re-exported here are the V1 types.
//! Rules that change them need versioned wrapper types.
//!
//! Epoch DA production (`DaAccumulatingState`) still encodes V1 DA payloads.
//! Rules that change the DA format must also select the encoder, not only the
//! decoder used by [`apply_da_epoch`] and [`verify_epoch_with_diff`].

mod block;
mod da;
mod selection;
pub mod sequencer;
mod spec;
mod v0;
mod v1;

pub use block::{
    construct_block, execute_and_complete_block, execute_block_batch_predrain, verify_block,
};
pub use da::{EpochDaReplayError, apply_da_epoch, verify_epoch_with_diff};
pub use selection::{
    EpochL1Range, EpochSpecSelectionError, InvalidEpochL1Range, UpgradeRequired, next_epoch_spec,
    select_next_epoch_spec,
};
pub use strata_ol_state_types::OLSpecId;
pub use strata_ol_stf_v1::{
    BasicExecContext, BlockComponents, BlockContext, BlockExecOutputs, BlockInfo, CompletedBlock,
    ConstructBlockOutput, EpochExecExpectations, EpochInfo, ExecError, ExecOutputBuffer,
    ExecResult, ManifestProcessingOutcome, TxExecContext,
};

#[cfg(test)]
mod tests;
