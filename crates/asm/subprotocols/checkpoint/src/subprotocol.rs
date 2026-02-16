//! Checkpoint Subprotocol Implementation

use strata_asm_checkpoint_msgs::CheckpointIncomingMsg;
use strata_asm_common::{
    AnchorState, AsmError, AuxRequestCollector, MsgRelayer, Subprotocol, SubprotocolId, TxInputRef,
    VerifiedAuxData, logging,
};
use strata_asm_params::CheckpointConfig;
use strata_asm_proto_checkpoint_txs::{
    CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE,
    parser::extract_signed_checkpoint_from_envelope,
};
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{handler::handle_checkpoint_tx, state::CheckpointState};

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
    const ID: SubprotocolId = CHECKPOINT_V0_SUBPROTOCOL_ID;

    type Params = CheckpointConfig;
    type State = CheckpointState;
    type Msg = CheckpointIncomingMsg;

    fn init(params: &Self::Params) -> Result<Self::State, AsmError> {
        Ok(CheckpointState::init(params.clone()))
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
                        let start_height = state.verified_tip().l1_height + 1;
                        let end_height = signed_checkpoint.inner().new_tip().l1_height;
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
        anchor_pre: &AnchorState,
        verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
    ) {
        // Calculate the current L1 height that this transaction is part of.
        // We add 1 because `anchor_pre` represents the prestate before applying
        // the new block. The `last_verified_block` is the previous block, so
        // the current block being processed is at height `last_verified_block + 1`.
        let current_l1_height = anchor_pre
            .chain_view
            .pow_state
            .last_verified_block
            .height_u32()
            + 1;

        for tx in txs {
            if tx.tag().tx_type() == OL_STF_CHECKPOINT_TX_TYPE {
                handle_checkpoint_tx(state, tx, current_l1_height, verified_aux_data, relayer)
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
