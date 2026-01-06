//! Checkpoint Subprotocol Implementation

use strata_asm_checkpoint_msgs::CheckpointIncomingMsg;
use strata_asm_common::{
    AnchorState, AsmError, AuxRequestCollector, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef,
    VerifiedAuxData, logging,
};
use strata_asm_proto_checkpoint_txs::{
    CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, extract_signed_checkpoint_from_envelope,
};

use crate::{
    handler::handle_checkpoint_tx,
    state::{CheckpointConfig, CheckpointState},
    utils::get_manifest_hash_range,
};

/// Checkpoint subprotocol implementation.
///
/// Implements the [`Subprotocol`] trait to integrate checkpoint verification
/// with the ASM. Responsibilities include:
///
/// - Processing checkpoint transactions (signature verification, proof verification)
/// - Validating state transitions (epoch, L1/L2 range progression)
/// - Forwarding withdrawal intents to the bridge subprotocol
/// - Processing configuration updates from the admin subprotocol
#[derive(Copy, Clone, Debug)]
pub struct CheckpointSubprotocol;

impl Subprotocol for CheckpointSubprotocol {
    const ID: SubprotocolId = CHECKPOINT_SUBPROTOCOL_ID;

    type Params = CheckpointConfig;
    type State = CheckpointState;
    type Msg = CheckpointIncomingMsg;

    fn init(params: &Self::Params) -> Result<Self::State, AsmError> {
        Ok(CheckpointState::new(params))
    }

    fn pre_process_txs(
        state: &Self::State,
        txs: &[TxInputRef<'_>],
        collector: &mut AuxRequestCollector,
        _anchor_pre: &AnchorState,
        _params: &Self::Params,
    ) {
        for tx in txs {
            if tx.tag().tx_type() == OL_STF_CHECKPOINT_TX_TYPE {
                match extract_signed_checkpoint_from_envelope(tx) {
                    Ok(signed_checkpoint) => {
                        let batch_info = &signed_checkpoint.inner.commitment.batch_info;
                        let (start_height, end_height) = get_manifest_hash_range(state, batch_info);
                        collector.request_manifest_hashes(start_height, end_height);
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
        _anchor_pre: &AnchorState,
        verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
    ) {
        for tx in txs {
            if tx.tag().tx_type() == OL_STF_CHECKPOINT_TX_TYPE {
                match handle_checkpoint_tx(state, tx, verified_aux_data, relayer) {
                    Ok(()) => {
                        logging::info!(
                            txid = %tx.tx().compute_txid(),
                            "Successfully processed checkpoint transaction"
                        );
                    }
                    Err(e) => {
                        logging::error!(
                            txid = %tx.tx().compute_txid(),
                            error = %e,
                            "Failed to process checkpoint transaction"
                        );
                    }
                }
            }
        }
    }

    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], _params: &Self::Params) {
        // ASM design assumes subprotocols are not adversarial against each other,
        // so no additional validation is performed on incoming messages.
        for msg in msgs {
            match msg {
                CheckpointIncomingMsg::UpdateSequencerKey(new_key) => {
                    logging::info!(%new_key, "Updating sequencer predicate");
                    state.update_sequencer_predicate(new_key.as_ref());
                }
                CheckpointIncomingMsg::UpdateCheckpointPredicate(new_predicate) => {
                    logging::info!("Updating checkpoint predicate");
                    state.update_checkpoint_predicate(new_predicate.clone());
                }
            }
        }
    }
}
