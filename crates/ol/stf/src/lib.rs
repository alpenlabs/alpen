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
//!
//! # Unknown and unimplemented specs
//!
//! Decoding rejects unknown identifiers, and dispatch matches every variant,
//! so a node never runs an epoch under older rules because it lacks newer
//! ones. Halting with upgrade instructions when a predicate enactment
//! activates a spec this binary does not know belongs to enactment discovery
//! (STR-4086), which must run before any operation here is called for the new
//! epoch. Until it lands, enactments do not advance the spec.
//!
//! [`OLSpecId::V0`] names the 0.3.0 rules, which networks launched on that
//! release run from genesis. This binary does not implement them yet
//! (STR-4486), so every operation under V0 returns
//! [`ExecError::UnimplementedSpec`] instead of running V0 epochs under V1
//! rules. Building genesis from V0 params therefore fails.
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
pub mod sequencer;
mod spec;
mod v1;

pub use block::{
    construct_block, execute_and_complete_block, execute_block_batch_predrain,
    validate_manifest_heights, verify_block, verify_header_continuity,
};
pub use da::{EpochDaReplayError, apply_da_epoch, verify_epoch_with_diff};
pub use strata_ol_state_types::OLSpecId;
pub use strata_ol_stf_v1::{
    BasicExecContext, BlockComponents, BlockContext, BlockExecOutputs, BlockInfo, CompletedBlock,
    ConstructBlockOutput, EpochExecExpectations, EpochInfo, ExecError, ExecOutputBuffer,
    ExecResult, ManifestProcessingOutcome, TxExecContext,
};

#[cfg(test)]
mod tests;
