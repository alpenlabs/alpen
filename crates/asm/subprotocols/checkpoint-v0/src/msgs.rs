use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_proto_checkpoint_txs::CHECKPOINT_V0_SUBPROTOCOL_ID;
use strata_primitives::{buf::Buf32, proof::RollupVerifyingKey};

/// Incoming messages that the checkpoint v0 subprotocol can receive from other subprotocols.
///
/// These messages are primarily emitted by the administration subprotocol to enact
/// configuration updates that originate from governance actions.
//
// Clippy warns about `large_enum_variant` here because the verifying key payload is big, but the
// checkpoint subprotocol needs to own the key without additional indirection. These messages
// are infrequent, so the extra size is an acceptable trade-off.
#[expect(
    clippy::large_enum_variant,
    reason = "checkpoint subprotocol needs to own the verifying key without indirection, messages are infrequent"
)]
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub enum CheckpointIncomingMsg {
    /// Update the Schnorr public key used to verify sequencer signatures embedded in checkpoints.
    UpdateSequencerKey(Buf32),
    /// Update the rollup proving system verifying key used for Groth16 proof verification.
    UpdateRollupVerifyingKey(RollupVerifyingKey),
}

impl InterprotoMsg for CheckpointIncomingMsg {
    fn id(&self) -> SubprotocolId {
        CHECKPOINT_V0_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn std::any::Any {
        self
    }
}
