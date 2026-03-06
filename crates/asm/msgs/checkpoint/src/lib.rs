//! Inter-protocol message types for the checkpoint subprotocol.
//!
//! This crate exposes the incoming message enum consumed by checkpoint subprotocols so other
//! subprotocols can send configuration updates or deposit notifications without depending on
//! the checkpoint implementation crate.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_txs_checkpoint::CHECKPOINT_SUBPROTOCOL_ID;
use strata_asm_txs_checkpoint_v0::CHECKPOINT_V0_SUBPROTOCOL_ID;
use strata_predicate::PredicateKey;
use strata_primitives::{buf::Buf32, l1::BitcoinAmount};

/// Incoming messages for checkpoint subprotocols.
///
/// Messages are routed to both the checkpoint-v0 and the new checkpoint.
/// Admin configuration updates target both, while deposit notifications
/// target the new checkpoint subprotocol.
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub enum CheckpointIncomingMsg {
    /// Update the Schnorr public key used to verify sequencer signatures embedded in checkpoints.
    // TODO: (@PG) make this directly take PredicateKey
    UpdateSequencerKey(Buf32),

    /// Update the rollup proving system verifying key used for Groth16 proof verification.
    UpdateCheckpointPredicate(PredicateKey),

    /// Notification that a deposit has been processed by the bridge subprotocol.
    DepositProcessed(BitcoinAmount),
}

impl InterprotoMsg for CheckpointIncomingMsg {
    fn id(&self) -> SubprotocolId {
        match self {
            // Admin config updates target checkpoint V0.
            Self::UpdateSequencerKey(_) | Self::UpdateCheckpointPredicate(_) => {
                CHECKPOINT_V0_SUBPROTOCOL_ID
            }
            // Deposit notifications target the new checkpoint subprotocol.
            Self::DepositProcessed(_) => CHECKPOINT_SUBPROTOCOL_ID,
        }
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}
