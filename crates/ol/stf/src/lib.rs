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
//! epoch it ends. Genesis always runs under [`OLSpecId::V1`].
//!
//! # Unknown specs
//!
//! An [`OLSpecId`] value always names rules this binary implements: decoding
//! rejects unknown identifiers, and dispatch matches every variant. A node
//! therefore never runs an epoch under older rules because it lacks newer
//! ones. Halting with upgrade instructions when a predicate enactment
//! activates a spec this binary does not know belongs to enactment discovery
//! (STR-4086), which must run before any operation here is called for the new
//! epoch. Until it lands, enactments do not advance the spec.
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
    construct_block, execute_and_complete_block, execute_block_batch_predrain, verify_block,
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
