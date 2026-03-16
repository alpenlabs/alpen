//! Inter-protocol message types for the checkpoint subprotocol.
//!
//! This crate exposes the incoming message enum consumed by checkpoint subprotocols so other
//! subprotocols can send configuration updates or deposit notifications without depending on
//! the checkpoint implementation crate.

use std::any::Any;

use ssz::{Decode, Encode};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_txs_checkpoint::CHECKPOINT_SUBPROTOCOL_ID;
use strata_asm_txs_checkpoint_v0::CHECKPOINT_V0_SUBPROTOCOL_ID;
use strata_btc_types::BitcoinAmount;
use strata_predicate::PredicateKey;
use strata_primitives::buf::Buf32;

#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use ssz_generated::ssz::messages::{
    CheckpointIncomingMsg, CheckpointIncomingMsgRef, DepositProcessed, DepositProcessedRef,
    PredicateBytes, UpdateCheckpointPredicate, UpdateCheckpointPredicateRef, UpdateSequencerKey,
    UpdateSequencerKeyRef,
};

fn encode_predicate(new_predicate: PredicateKey) -> PredicateBytes {
    PredicateBytes::new(new_predicate.as_ssz_bytes())
        .expect("checkpoint predicate must stay within SSZ bounds")
}

fn decode_predicate(bytes: &[u8]) -> PredicateKey {
    PredicateKey::from_ssz_bytes(bytes).expect("checkpoint predicate bytes must remain valid")
}

impl UpdateSequencerKey {
    /// Creates a sequencer-key update payload.
    pub fn new(new_key: Buf32) -> Self {
        Self {
            new_key: new_key.into(),
        }
    }

    /// Returns the new sequencer key.
    pub fn new_key(&self) -> Buf32 {
        let key_bytes: [u8; 32] = self
            .new_key
            .as_ref()
            .try_into()
            .expect("checkpoint sequencer key must remain 32 bytes");
        key_bytes.into()
    }
}

impl DepositProcessed {
    /// Creates a deposit-processed payload.
    pub fn new(amount: BitcoinAmount) -> Self {
        Self { amount }
    }

    /// Returns the processed deposit amount.
    pub fn amount(&self) -> BitcoinAmount {
        self.amount
    }
}

impl UpdateCheckpointPredicate {
    /// Creates a checkpoint-predicate update payload.
    pub fn new(new_predicate: PredicateKey) -> Self {
        Self {
            new_predicate: encode_predicate(new_predicate),
        }
    }

    /// Returns the updated checkpoint predicate.
    pub fn new_predicate(&self) -> PredicateKey {
        decode_predicate(&self.new_predicate)
    }
}

impl CheckpointIncomingMsg {
    /// Creates a sequencer-key update message.
    pub fn update_sequencer_key(new_key: Buf32) -> Self {
        Self::UpdateSequencerKey(UpdateSequencerKey::new(new_key))
    }

    /// Creates a checkpoint-predicate update message.
    pub fn update_checkpoint_predicate(new_predicate: PredicateKey) -> Self {
        Self::UpdateCheckpointPredicate(UpdateCheckpointPredicate::new(new_predicate))
    }

    /// Creates a deposit-processed notification message.
    pub fn deposit_processed(amount: BitcoinAmount) -> Self {
        Self::DepositProcessed(DepositProcessed::new(amount))
    }
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
