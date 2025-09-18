//! Checkpointing v0 Subprotocol Implementation
//!
//! This module implements the checkpointing v0 subprotocol that maintains feature parity
//! with the current checkpointing system while following SPS-62 structure where beneficial.
//!
//! NOTE: This implementation bridges current checkpoint format with SPS-62 concepts,
//! focusing on compatibility with existing CSM checkpoint verification.

use strata_asm_common::{
    logging, AnchorState, AsmError, MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInputRef,
};
use strata_asm_proto_bridge_v1::{BridgeIncomingMsg, WithdrawOutput};

use crate::{
    constants::CHECKPOINTING_V0_SUBPROTOCOL_ID,
    error::{CheckpointV0Error, CheckpointV0Result},
    parsing::{extract_signed_checkpoint_from_envelope, extract_withdrawal_messages},
    types::{CheckpointV0VerificationParams, CheckpointV0VerifierState},
    verification::process_checkpoint_v0,
};

/// Checkpointing v0 subprotocol configuration
///
/// NOTE: This maintains compatibility with current checkpoint parameters while
/// incorporating SPS-62 structure concepts for future transition
#[derive(Clone, Debug)]
pub struct CheckpointingV0Config {
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
    type Params = CheckpointingV0Config;
    type Msg = NullMsg<CHECKPOINTING_V0_SUBPROTOCOL_ID>;
    type AuxInput = ();

    fn init(params: &Self::Params) -> Result<Self::State, AsmError> {
        let genesis_l1_block = params.verification_params.genesis_l1_block;
        Ok(CheckpointV0VerifierState::new_genesis(genesis_l1_block))
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
        params: &Self::Params,
    ) {
        // Get current L1 height from anchor state
        let current_l1_height = anchor_pre.chain_view.pow_state.last_verified_block.height();

        for tx in txs {
            let result =
                process_checkpoint_transaction_v0(state, tx, current_l1_height, relayer, params);

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

    /// Process incoming messages (not used in checkpointing v0)
    ///
    /// Currently, the checkpointing v0 subprotocol uses NullMsg and does not
    /// process incoming messages. All checkpoint verification is transaction-driven.
    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg], _params: &Self::Params) {
        // Checkpointing v0 doesn't process inter-subprotocol messages
        // All functionality is transaction-based
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
/// NOTE: This bridges current checkpoint format with SPS-50 envelope parsing
fn process_checkpoint_transaction_v0(
    state: &mut CheckpointV0VerifierState,
    tx: &TxInputRef<'_>,
    current_l1_height: u64,
    relayer: &mut impl MsgRelayer,
    params: &CheckpointingV0Config,
) -> CheckpointV0Result<bool> {
    // 1. Extract signed checkpoint from SPS-50 envelope
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;

    // 3. Process checkpoint using v0 verification logic
    let accepted = process_checkpoint_v0(state, &signed_checkpoint, &params.verification_params)
        .map_err(|e| CheckpointV0Error::StateTransitionError(e.to_string()))?;

    if accepted {
        // 4. Extract withdrawal messages for bridge forwarding
        let withdrawal_intents = extract_withdrawal_messages(signed_checkpoint.checkpoint())
            .map_err(|e| CheckpointV0Error::StateTransitionError(e.to_string()))?;

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
    }

    Ok(accepted)
}
