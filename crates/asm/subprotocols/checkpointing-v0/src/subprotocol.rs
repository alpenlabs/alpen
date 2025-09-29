//! Checkpointing v0 Subprotocol Implementation
//!
//! This module implements the checkpointing v0 subprotocol that maintains feature parity
//! with the current checkpointing system while following SPS-62 structure where beneficial.
//!
//! NOTE: This implementation bridges the legacy checkpoint payload format with the new SPS-50
//! envelope layout so we can reuse existing verification logic while moving toward SPS-62.

use strata_asm_common::{
    logging, AnchorState, AsmError, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef,
};
use strata_asm_proto_bridge_v1::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_proto_checkpointing_txs::{
    extract_signed_checkpoint_from_envelope, extract_withdrawal_messages,
    CHECKPOINTING_V0_SUBPROTOCOL_ID,
};
use strata_primitives::{block_credential::CredRule, buf::Buf32, proof::RollupVerifyingKey};

use crate::{
    error::{CheckpointV0Error, CheckpointV0Result},
    msgs::CheckpointingIncomingMsg,
    types::{CheckpointV0VerificationParams, CheckpointV0VerifierState},
    verification::process_checkpoint_v0,
};

/// Checkpointing v0 subprotocol parameters
///
/// NOTE: This maintains compatibility with current checkpoint parameters while
/// incorporating SPS-62 structure concepts for future transition
#[derive(Clone, Debug)]
pub struct CheckpointingV0Params {
    /// Verification parameters for checkpoint validation
    pub verification_params: CheckpointV0VerificationParams,
}

/// Checkpointing v0 subprotocol implementation.
///
/// This struct implements the [`Subprotocol`] trait to integrate checkpoint
/// verification functionality with the ASM. It maintains feature parity with
/// the current checkpointing system while following SPS-62 envelope structure.
///
/// NOTE: This is checkpointing v0 - focused on current system compatibility.
/// Future versions will be fully SPS-62 compliant.
#[derive(Copy, Clone, Debug)]
pub struct CheckpointingV0Subproto;

impl Subprotocol for CheckpointingV0Subproto {
    const ID: SubprotocolId = CHECKPOINTING_V0_SUBPROTOCOL_ID;

    type State = CheckpointV0VerifierState;
    type Params = CheckpointingV0Params;
    type Msg = CheckpointingIncomingMsg;
    type AuxInput = ();

    fn init(params: &Self::Params) -> Result<Self::State, AsmError> {
        Ok(CheckpointV0VerifierState::new(&params.verification_params))
    }

    /// Process checkpoint transactions according to checkpointing v0 specification
    ///
    /// This function handles checkpoint transactions that use current checkpoint format
    /// wrapped in SPS-50 envelopes:
    /// 1. Parse SPS-50 envelope transactions (following administration pattern)
    /// 2. Extract signed checkpoints in current format
    /// 3. Verify checkpoints using bridge to current verification system
    /// 4. Update internal verifier state
    /// 5. Extract and forward withdrawal messages to bridge subprotocol
    ///
    /// NOTE: This maintains feature parity with current checkpoint verification
    /// while using SPS-50 envelope parsing for future compatibility.
    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
        _aux_input: &Self::AuxInput,
        relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
    ) {
        // Get current L1 height from anchor state
        let current_l1_height = anchor_pre.chain_view.pow_state.last_verified_block.height();

        for tx in txs {
            let result = process_checkpoint_transaction_v0(state, tx, current_l1_height, relayer);

            // Log transaction processing results
            match result {
                Ok(accepted) => {
                    if accepted {
                        let txid = tx.tx().compute_txid();
                        logging::info!("Successfully processed checkpoint transaction: {txid:?}");
                    } else {
                        let txid = tx.tx().compute_txid();
                        logging::warn!("Rejected checkpoint transaction: {txid:?}");
                    }
                }
                Err(e) => {
                    let txid = tx.tx().compute_txid();
                    logging::warn!("Error processing checkpoint transaction {txid:?}: {e:?}");
                }
            }
        }
    }

    /// Process incoming administration upgrade messages for checkpointing v0.
    ///
    /// Handles configuration updates emitted by the administration subprotocol such as
    /// sequencer key rotations and rollup verifying key refreshes.
    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], _params: &Self::Params) {
        for msg in msgs {
            match msg {
                CheckpointingIncomingMsg::UpdateSequencerKey(new_key) => {
                    apply_sequencer_update(state, *new_key);
                }
                CheckpointingIncomingMsg::UpdateRollupVerifyingKey(new_vk) => {
                    apply_rollup_vk_update(state, new_vk.clone());
                }
            }
        }
    }
}

/// Process a single checkpoint transaction (v0 implementation)
///
/// This function implements the core checkpoint transaction processing:
/// 1. Extract signed checkpoint from SPS-50 envelope (following admin pattern)
/// 2. Create verification context from transaction and anchor state
/// 3. Delegate to checkpoint verification logic
/// 4. Extract and forward withdrawal messages if checkpoint is accepted
///
/// NOTE: The inner checkpoint payload still uses the legacy (TN1) format; the transaction wrapper
/// follows SPS-50 so legacy verification can run while we migrate to the SPS-62 format.
fn process_checkpoint_transaction_v0(
    state: &mut CheckpointV0VerifierState,
    tx: &TxInputRef<'_>,
    current_l1_height: u64,
    relayer: &mut impl MsgRelayer,
) -> CheckpointV0Result<bool> {
    // 1. Extract signed checkpoint from SPS-50 envelope
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;

    // 2. Process checkpoint using v0 verification logic
    process_checkpoint_v0(state, &signed_checkpoint, current_l1_height)?;

    // 3. Extract withdrawal messages for bridge forwarding. The OL state machine already tracks
    //    withdrawal intents, but the bridge subprotocol still needs an explicit message so it can
    //    drive Bitcoin-side assignment without re-parsing checkpoints.
    let withdrawal_intents = extract_withdrawal_messages(signed_checkpoint.checkpoint())
        .map_err(|e| CheckpointV0Error::ParsingError(e.to_string()))?;

    // Forward each withdrawal message to the bridge subprotocol
    for intent in withdrawal_intents {
        let withdraw_output = WithdrawOutput::new(intent.destination().clone(), *intent.amt());
        // Wrap it in [`BridgeIncomingMsg`]
        let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);

        // Send to bridge subprotocol
        relayer.relay_msg(&bridge_msg);
    }

    let epoch = signed_checkpoint.checkpoint().batch_info().epoch();
    logging::info!(
        "Checkpoint accepted for epoch {}, L1 height {}",
        epoch,
        current_l1_height
    );

    Ok(true)
}

fn apply_sequencer_update(state: &mut CheckpointV0VerifierState, new_key: Buf32) {
    let previous_rule = state.cred_rule.clone();

    if matches!(&previous_rule, CredRule::SchnorrKey(existing) if existing == &new_key) {
        logging::info!("Sequencer key update received, key unchanged");
        return;
    }

    state.update_sequencer_key(new_key);

    match previous_rule {
        CredRule::SchnorrKey(_) => {
            logging::info!(new_key = %new_key, "Updated sequencer public key")
        }
        CredRule::Unchecked => {
            logging::warn!(new_key = %new_key, "Enabled sequencer key checks via admin update")
        }
    }
}

fn apply_rollup_vk_update(state: &mut CheckpointV0VerifierState, new_vk: RollupVerifyingKey) {
    let prev_kind = rollup_vk_kind(&state.rollup_verifying_key);
    let next_kind = rollup_vk_kind(&new_vk);

    if prev_kind == next_kind {
        // Even if the proving system stays the same we still replace the key to pick up new
        // parameters.
        logging::info!(kind = next_kind, "Applying rollup verifying key update");
    } else {
        logging::info!(
            previous = prev_kind,
            next = next_kind,
            "Switching rollup verifying key proving system"
        );
    }

    state.update_rollup_verifying_key(new_vk);
}

fn rollup_vk_kind(vk: &RollupVerifyingKey) -> &'static str {
    match vk {
        RollupVerifyingKey::SP1VerifyingKey(_) => "sp1",
        RollupVerifyingKey::Risc0VerifyingKey(_) => "risc0",
        RollupVerifyingKey::NativeVerifyingKey => "native",
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::{
        block_credential::CredRule,
        buf::Buf32,
        l1::{L1BlockCommitment, L1BlockId},
        params::ProofPublishMode,
        proof::RollupVerifyingKey,
    };

    use super::*;

    fn test_params() -> CheckpointingV0Params {
        let genesis_commitment = L1BlockCommitment::new(0, L1BlockId::from(Buf32::default()));
        let verification_params = CheckpointV0VerificationParams {
            genesis_l1_block: genesis_commitment,
            cred_rule: CredRule::Unchecked,
            rollup_verifying_key: RollupVerifyingKey::NativeVerifyingKey,
            proof_publish_mode: ProofPublishMode::Strict,
        };

        CheckpointingV0Params {
            verification_params,
        }
    }

    #[test]
    fn process_msgs_updates_sequencer_key() {
        let params = test_params();
        let mut state = CheckpointingV0Subproto::init(&params).expect("init state");

        let new_key = Buf32::from([42u8; 32]);
        let msgs = [CheckpointingIncomingMsg::UpdateSequencerKey(new_key)];

        CheckpointingV0Subproto::process_msgs(&mut state, &msgs, &params);

        match &state.cred_rule {
            CredRule::SchnorrKey(current) => assert_eq!(current, &new_key),
            CredRule::Unchecked => panic!("sequencer key should switch to schnorr rule"),
        }
    }

    #[test]
    fn process_msgs_updates_rollup_verifying_key() {
        let params = test_params();
        let mut state = CheckpointingV0Subproto::init(&params).expect("init state");

        let msgs = [CheckpointingIncomingMsg::UpdateRollupVerifyingKey(
            RollupVerifyingKey::NativeVerifyingKey,
        )];

        CheckpointingV0Subproto::process_msgs(&mut state, &msgs, &params);

        assert!(matches!(
            state.rollup_verifying_key,
            RollupVerifyingKey::NativeVerifyingKey
        ));
    }
}
