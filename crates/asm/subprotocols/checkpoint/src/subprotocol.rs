//! Checkpoint Subprotocol Implementation
//!
//! This module implements the `Subprotocol` trait for checkpoint verification.

use strata_asm_checkpoint_msgs::CheckpointIncomingMsg;
use strata_asm_common::{
    AnchorState, AsmError, AuxRequestCollector, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef,
    VerifiedAuxData, logging,
};
use strata_asm_proto_checkpoint_txs::{
    CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE,
    extract_signed_checkpoint_from_envelope,
};

use crate::{
    handler::handle_checkpoint_tx,
    msg_handler::{apply_predicate_update, apply_sequencer_key_update},
    state::{CheckpointConfig, CheckpointState},
};

/// Checkpoint subprotocol implementation.
///
/// Implements the [`Subprotocol`] trait to integrate checkpoint verification
/// with the ASM. Handles checkpoint transactions, proof verification, and
/// inter-protocol messages for configuration updates.
#[derive(Copy, Clone, Debug)]
pub struct CheckpointSubprotocol;

impl Subprotocol for CheckpointSubprotocol {
    const ID: SubprotocolId = CHECKPOINT_V0_SUBPROTOCOL_ID;

    type State = CheckpointState;
    type Params = CheckpointConfig;
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
            if tx.tag().tx_type() != OL_STF_CHECKPOINT_TX_TYPE {
                continue;
            }

            let start_height = state.last_checkpoint_l1().height_u64();

            // Parse checkpoint to get exact L1 range for manifest hash request
            match extract_signed_checkpoint_from_envelope(tx) {
                Ok(signed_checkpoint) => {
                    let batch_info = signed_checkpoint.payload().batch_info();
                    let end_height = batch_info.final_l1_block().height_u64();
                    collector.request_manifest_hashes(start_height, end_height);
                }
                Err(e) => {
                    logging::warn!(
                        txid = %tx.tx().compute_txid(),
                        error = ?e,
                        "Failed to parse checkpoint in pre-process phase"
                    )
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
            if tx.tag().tx_type() != OL_STF_CHECKPOINT_TX_TYPE {
                logging::debug!(
                    txid = %tx.tx().compute_txid(),
                    tx_type = tx.tag().tx_type(),
                    "Skipping non-checkpoint transaction"
                );
                continue;
            }

            match handle_checkpoint_tx(state, tx, verified_aux_data, relayer) {
                Ok(()) => {
                    logging::info!(
                        txid = %tx.tx().compute_txid(),
                        "Successfully processed checkpoint transaction"
                    );
                }
                Err(error) => {
                    logging::warn!(
                        txid = %tx.tx().compute_txid(),
                        error = ?error,
                        "Failed to process checkpoint transaction"
                    );
                }
            }
        }
    }

    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], _params: &Self::Params) {
        for msg in msgs {
            match msg {
                CheckpointIncomingMsg::UpdateSequencerKey(new_key) => {
                    apply_sequencer_key_update(state, *new_key);
                }
                CheckpointIncomingMsg::UpdateCheckpointPredicate(new_predicate) => {
                    apply_predicate_update(state, new_predicate);
                }
            }
        }
    }
}
