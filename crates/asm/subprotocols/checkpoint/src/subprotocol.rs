//! Checkpoint Subprotocol Implementation

use strata_asm_checkpoint_msgs::CheckpointIncomingMsg;
use strata_asm_common::{
    AuxRequestCollector, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef, VerifiedAuxData,
    logging,
};
use strata_asm_params::CheckpointInitConfig;
use strata_asm_txs_checkpoint::{
    CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, extract_checkpoint_from_envelope,
};
use strata_identifiers::L1BlockCommitment;
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{handler::handle_checkpoint_tx, state::CheckpointState};

/// Checkpoint subprotocol implementation.
///
/// Implements the [`Subprotocol`] trait to integrate checkpoint verification
/// with the ASM. Responsibilities include:
///
/// - Processing checkpoint transactions (envelope pubkey verification, proof verification)
/// - Validating state transitions (epoch, L1/L2 range progression)
/// - Forwarding withdrawal intents to the bridge subprotocol
/// - Processing configuration updates from the admin subprotocol
#[derive(Copy, Clone, Debug)]
pub struct CheckpointSubprotocol;

impl Subprotocol for CheckpointSubprotocol {
    const ID: SubprotocolId = CHECKPOINT_SUBPROTOCOL_ID;

    type InitConfig = CheckpointInitConfig;
    type State = CheckpointState;
    type Msg = CheckpointIncomingMsg;

    fn init(config: &Self::InitConfig) -> Self::State {
        CheckpointState::init(config.clone())
    }

    fn pre_process_txs(
        state: &Self::State,
        txs: &[TxInputRef<'_>],
        collector: &mut AuxRequestCollector,
    ) {
        for tx in txs {
            if tx.tag().tx_type() == OL_STF_CHECKPOINT_TX_TYPE {
                match extract_checkpoint_from_envelope(tx) {
                    Ok(envelope) => {
                        let start_height = state.verified_tip().l1_height + 1;
                        let end_height = envelope.payload.new_tip().l1_height;
                        collector.request_manifest_hashes(start_height as u64, end_height as u64);
                    }
                    Err(e) => {
                        logging::warn!(
                            txid = ?tx.tx().compute_txid(),
                            error = ?e,
                            "Failed to parse checkpoint transaction in pre_process_txs"
                        );
                    }
                }
            }
        }
    }

    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        l1ref: &L1BlockCommitment,
        verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
    ) {
        let current_l1_height = l1ref.height();

        for tx in txs {
            if tx.tag().tx_type() == OL_STF_CHECKPOINT_TX_TYPE {
                handle_checkpoint_tx(state, tx, current_l1_height, verified_aux_data, relayer)
            }
        }
    }

    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], _l1ref: &L1BlockCommitment) {
        // ASM design assumes subprotocols are not adversarial against each other,
        // so no additional validation is performed on incoming messages.
        for msg in msgs {
            match msg {
                CheckpointIncomingMsg::DepositProcessed(amount) => {
                    logging::info!(amount_sat = amount.to_sat(), "Recording processed deposit");
                    state.record_deposit(*amount);
                }
                CheckpointIncomingMsg::UpdateSequencerKey(new_key) => {
                    logging::info!(%new_key, "Updating sequencer predicate");
                    let new_predicate_key =
                        PredicateKey::new(PredicateTypeId::Bip340Schnorr, new_key.0.to_vec());
                    state.update_sequencer_predicate(new_predicate_key);
                }
                CheckpointIncomingMsg::UpdateCheckpointPredicate(new_predicate) => {
                    logging::info!("Updating checkpoint predicate");
                    state.update_checkpoint_predicate(new_predicate.clone());
                }
            }
        }
    }
}
