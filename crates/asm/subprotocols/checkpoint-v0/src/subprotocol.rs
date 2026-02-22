//! Checkpoint v0 Subprotocol Implementation
//!
//! This module implements the checkpoint v0 subprotocol that maintains feature parity
//! with the current checkpoint system while following SPS-62 structure where beneficial.
//!
//! NOTE: This implementation bridges the legacy checkpoint payload format with the new SPS-50
//! envelope layout so we can reuse existing verification logic while moving toward SPS-62.

use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_checkpoint_msgs::CheckpointIncomingMsg;
use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef,
    VerifiedAuxData, logging,
};
use strata_asm_logs::CheckpointUpdate;
use strata_asm_txs_checkpoint_v0::{
    CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE,
    extract_signed_checkpoint_from_envelope, extract_withdrawal_messages,
};
use strata_ol_msg_types::resolve_selected_operator;
use strata_predicate::PredicateKey;
use strata_primitives::{block_credential::CredRule, buf::Buf32, l1::BitcoinTxid};

use crate::{
    error::{CheckpointV0Error, CheckpointV0Result},
    types::{CheckpointV0VerificationParams, CheckpointV0VerifierState},
    verification::process_checkpoint_v0,
};

/// Checkpoint v0 subprotocol parameters
///
/// NOTE: This maintains compatibility with current checkpoint parameters while
/// incorporating SPS-62 structure concepts for future transition
#[derive(Clone, Debug)]
pub struct CheckpointV0Params {
    /// Verification parameters for checkpoint validation
    pub verification_params: CheckpointV0VerificationParams,
}

/// Checkpoint v0 subprotocol implementation.
///
/// This struct implements the [`Subprotocol`] trait to integrate checkpoint
/// verification functionality with the ASM. It maintains feature parity with
/// the current checkpoint system while following SPS-62 envelope structure.
///
/// NOTE: This is checkpoint v0 - focused on current system compatibility.
/// Future versions will be fully SPS-62 compliant.
#[derive(Copy, Clone, Debug)]
pub struct CheckpointV0Subproto;

impl Subprotocol for CheckpointV0Subproto {
    const ID: SubprotocolId = CHECKPOINT_V0_SUBPROTOCOL_ID;

    type State = CheckpointV0VerifierState;
    type Params = CheckpointV0Params;
    type Msg = CheckpointIncomingMsg;

    fn init(params: &Self::Params) -> Result<Self::State, AsmError> {
        Ok(CheckpointV0VerifierState::new(&params.verification_params))
    }

    /// Process checkpoint transactions according to checkpoint v0 specification
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
        _verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
    ) {
        // Get current L1 height from anchor state
        let current_l1_height = anchor_pre.chain_view.pow_state.last_verified_block.height();
        let current_l1_height_u64 = current_l1_height.to_consensus_u32() as u64;

        for tx in txs {
            let tx_type = tx.tag().tx_type();
            if tx_type != OL_STF_CHECKPOINT_TX_TYPE {
                logging::debug!(
                    txid = %tx.tx().compute_txid(),
                    tx_type,
                    "Skipping non-checkpoint transaction in checkpoint subprotocol",
                );
                continue;
            }

            match process_checkpoint_transaction_v0(state, tx, current_l1_height_u64, relayer) {
                Ok(true) => {
                    logging::info!(
                        txid = %tx.tx().compute_txid(),
                        "Successfully processed checkpoint transaction"
                    );
                }
                Ok(false) => {
                    logging::warn!(
                        txid = %tx.tx().compute_txid(),
                        "Rejected checkpoint transaction"
                    );
                }
                Err(error) => {
                    logging::warn!(
                        txid = %tx.tx().compute_txid(),
                        error = ?error,
                        "Error processing checkpoint transaction"
                    );
                }
            }
        }
    }

    /// Process incoming administration upgrade messages for checkpoint v0.
    ///
    /// Handles configuration updates emitted by the administration subprotocol such as
    /// sequencer key rotations and rollup verifying key refreshes.
    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], _params: &Self::Params) {
        for msg in msgs {
            match msg {
                CheckpointIncomingMsg::UpdateSequencerKey(new_key) => {
                    apply_sequencer_update(state, *new_key);
                }
                CheckpointIncomingMsg::UpdateCheckpointPredicate(new_predicate) => {
                    apply_rollup_vk_update(state, new_predicate);
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

    // we only have one tx type for checkpoint v0
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;

    // 2. Process checkpoint using v0 verification logic
    process_checkpoint_v0(state, &signed_checkpoint, current_l1_height)?;

    // 3. Extract withdrawal messages for bridge forwarding.
    let withdrawal_intents = extract_withdrawal_messages(signed_checkpoint.checkpoint())
        .map_err(|e| CheckpointV0Error::ParsingError(e.to_string()))?;

    // Forward each withdrawal message to the bridge subprotocol
    for intent in withdrawal_intents {
        let selected_operator = resolve_selected_operator(intent.selected_operator());
        let withdraw_output = WithdrawOutput::new(
            intent.destination().clone(),
            *intent.amt(),
            selected_operator,
        );
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

    // Emit CheckpointUpdate log
    let checkpoint_txid = BitcoinTxid::new(&tx.tx().compute_txid());
    let checkpoint_update =
        CheckpointUpdate::from_checkpoint(signed_checkpoint.checkpoint(), checkpoint_txid);

    match AsmLogEntry::from_log(&checkpoint_update) {
        Ok(log_entry) => relayer.emit_log(log_entry),
        Err(err) => logging::error!(error = ?err, "Failed to encode checkpoint update log"),
    }

    Ok(true)
}

fn apply_sequencer_update(state: &mut CheckpointV0VerifierState, new_key: Buf32) {
    let previous_rule = state.cred_rule.clone();

    if matches!(&previous_rule, CredRule::SchnorrKey(existing) if existing == &new_key) {
        logging::info!("Sequencer key update received, key unchanged");
        return;
    }

    match previous_rule {
        CredRule::SchnorrKey(_) => {
            state.update_sequencer_key(new_key);
            logging::info!(new_key = %new_key, "Updated sequencer schnorr public key");
        }
        CredRule::Unchecked => {
            logging::warn!(new_key = %new_key, "Received unchecked sequencer key update from administration");
            state.update_sequencer_key(new_key);
            logging::info!(new_key = %new_key, "Updated sequencer public key to unchecked CredRule");
        }
    }
}

fn apply_rollup_vk_update(state: &mut CheckpointV0VerifierState, new_predicate: &PredicateKey) {
    let prev_kind = state.predicate.id();
    let next_kind = new_predicate.id();

    if prev_kind == next_kind {
        logging::info!(kind = %next_kind, "Applying rollup verifying key update");
    } else {
        logging::info!(
            previous = %prev_kind,
            next = %next_kind,
            "Switching rollup proving system"
        );
    }

    state.update_predicate(new_predicate.clone());
}

#[cfg(test)]
mod tests {
    use strata_primitives::{
        block_credential::CredRule,
        buf::Buf32,
        l1::{L1BlockCommitment, L1BlockId},
    };

    use super::*;

    fn test_params() -> CheckpointV0Params {
        let genesis_commitment =
            L1BlockCommitment::from_height_u64(0, L1BlockId::from(Buf32::default()))
                .expect("genesis height should be valid");
        let verification_params = CheckpointV0VerificationParams {
            genesis_l1_block: genesis_commitment,
            cred_rule: CredRule::Unchecked,
            predicate: PredicateKey::always_accept(),
        };

        CheckpointV0Params {
            verification_params,
        }
    }

    #[test]
    fn process_msgs_updates_sequencer_key() {
        let params = test_params();
        let mut state = CheckpointV0Subproto::init(&params).expect("init state");

        let new_key = Buf32::from([42u8; 32]);
        let msgs = [CheckpointIncomingMsg::UpdateSequencerKey(new_key)];

        CheckpointV0Subproto::process_msgs(&mut state, &msgs, &params);

        match &state.cred_rule {
            CredRule::SchnorrKey(current) => assert_eq!(current, &new_key),
            CredRule::Unchecked => panic!("sequencer key should switch to schnorr rule"),
        }
    }
}
