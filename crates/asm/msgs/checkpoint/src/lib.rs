//! Inter-protocol message types for the checkpoint subprotocol.
//!
//! This crate exposes the incoming message enum consumed by checkpoint-v0 so other
//! subprotocols can send configuration updates without depending on the checkpoint
//! implementation crate.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_proto_checkpoint_txs::CHECKPOINT_SUBPROTOCOL_ID;
use strata_predicate::PredicateKey;
use strata_primitives::buf::Buf32;

/// Incoming messages that the checkpoint subprotocol can receive from other subprotocols.
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub enum CheckpointIncomingMsg {
    /// Update the Schnorr public key used to verify sequencer signatures embedded in checkpoints.
    UpdateSequencerKey(Buf32),
    /// Update the rollup proving system verifying key used for Groth16 proof verification.
    UpdateCheckpointPredicate(PredicateKey),
}

impl InterprotoMsg for CheckpointIncomingMsg {
    fn id(&self) -> SubprotocolId {
        CHECKPOINT_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}
