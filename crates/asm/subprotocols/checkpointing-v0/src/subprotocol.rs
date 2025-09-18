//! Checkpointing v0 Subprotocol Implementation
//!
//! This module implements the checkpointing v0 subprotocol that maintains feature parity
//! with the current checkpointing system while following SPS-62 structure where beneficial.
//!
//! NOTE: This implementation bridges current checkpoint format with SPS-62 concepts,
//! focusing on compatibility with existing CSM checkpoint verification.

use strata_asm_common::{
    logging, AnchorState, AsmError, AuxInputCollector, MsgRelayer, NullMsg, Subprotocol,
    SubprotocolId, TxInputRef,
};

use crate::{
    constants::CHECKPOINTING_V0_SUBPROTOCOL_ID,
    error::{CheckpointV0Error, CheckpointV0Result},
    parsing::extract_signed_checkpoint_from_envelope,
    types::{CheckpointV0AuxInput, CheckpointV0VerificationParams, CheckpointV0VerifierState},
    verification::{extract_withdrawal_messages, process_checkpoint_v0},
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
    type AuxInput = CheckpointV0AuxInput;

    fn init(params: &Self::Params) -> Result<Self::State, AsmError> {
        let genesis_l1_block = params.verification_params.genesis_l1_block;
        Ok(CheckpointV0VerifierState::new_genesis(genesis_l1_block))
    }

    fn pre_process_txs(
        _state: &Self::State,
        _txs: &[TxInputRef<'_>],
        _collector: &mut impl AuxInputCollector,
        _anchor_pre: &AnchorState,
        _params: &Self::Params,
    ) {
        // NOTE: For checkpointing v0, auxiliary input collection is simplified.
        // Future versions will collect full L1 oracle data as per SPS-62.
        //
        // In full SPS-62 implementation, this would collect:
        // - L1 block height/ID oracles
        // - L1 manifest oracles for historical state
        // - Other auxiliary data required for verification
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
        aux_input: &Self::AuxInput,
        relayer: &mut impl MsgRelayer,
        params: &Self::Params,
    ) {
        // Get current L1 height from anchor state
        let current_l1_height = anchor_pre.chain_view.pow_state.last_verified_block.height();

        for tx in txs {
            let result = process_checkpoint_transaction_v0(
                state,
                tx,
                current_l1_height,
                aux_input,
                relayer,
                params,
            );

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
    aux_input: &CheckpointV0AuxInput,
    _relayer: &mut impl MsgRelayer,
    params: &CheckpointingV0Config,
) -> CheckpointV0Result<bool> {
    // 1. Extract signed checkpoint from SPS-50 envelope
    let (signed_checkpoint, mut verify_context) = extract_signed_checkpoint_from_envelope(tx)?;

    // 2. Update verification context with current L1 height from anchor state
    verify_context.current_l1_height = current_l1_height;

    // 3. Process checkpoint using v0 verification logic
    let accepted = process_checkpoint_v0(
        state,
        &signed_checkpoint,
        &verify_context,
        aux_input,
        &params.verification_params,
    )
    .map_err(|e| CheckpointV0Error::StateTransitionError(e.to_string()))?;

    if accepted {
        // 4. Extract withdrawal messages for bridge forwarding
        let _withdrawal_messages = extract_withdrawal_messages(signed_checkpoint.checkpoint())
            .map_err(|e| CheckpointV0Error::StateTransitionError(e.to_string()))?;

        // TODO: Forward withdrawal messages to bridge subprotocol
        // In the full implementation, this would:
        // - Convert withdrawal messages to bridge-compatible format
        // - Send messages to bridge subprotocol via relayer
        // - Emit checkpoint acceptance log
        //
        // For v0, we skip bridge integration as a placeholder

        let epoch = signed_checkpoint.checkpoint().batch_info().epoch();
        logging::info!(
            "Checkpoint accepted for epoch {}, L1 height {}",
            epoch,
            current_l1_height
        );
    }

    Ok(accepted)
}

/// Create auxiliary input for verification (simplified for v0)
///
/// This constructs simplified auxiliary input for checkpoint verification.
/// Full SPS-62 implementation would collect comprehensive L1 oracle data.
#[allow(dead_code)] // Reserved for future use
fn create_aux_input_v0(anchor_pre: &AnchorState) -> CheckpointV0AuxInput {
    // For checkpointing v0, create simplified auxiliary input
    let current_l1_height = anchor_pre.chain_view.pow_state.last_verified_block.height();
    let current_l1_blkid = anchor_pre.chain_view.pow_state.last_verified_block.blkid();

    CheckpointV0AuxInput {
        current_l1_height,
        current_l1_blkid: (*current_l1_blkid).into(),
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};

    use super::*;
    use crate::types::*;

    fn create_test_config() -> CheckpointingV0Config {
        let verification_params = CheckpointV0VerificationParams {
            sequencer_pubkey: Buf32::zero(),
            skip_proof_verification: true, // For testing
            genesis_l1_block: L1BlockCommitment::new(0, Buf32::zero().into()),
            rollup_verifying_key: None, // No verifying key needed for test
        };

        CheckpointingV0Config {
            verification_params,
        }
    }

    #[test]
    fn test_subprotocol_init() {
        let config = create_test_config();
        let result = CheckpointingV0Subproto::init(&config);

        assert!(result.is_ok());
        let state = result.unwrap();
        assert_eq!(state.current_epoch(), 0);
        assert_eq!(state.last_checkpoint_l1_height, 0);
    }

    #[test]
    fn test_aux_input_creation() {
        // Test auxiliary input structure
        let aux_input = CheckpointV0AuxInput {
            current_l1_height: 100,
            current_l1_blkid: Buf32::zero(),
        };

        assert_eq!(aux_input.current_l1_height, 100);
    }

    #[test]
    fn test_config_structure() {
        let config = create_test_config();
        assert!(config.verification_params.skip_proof_verification);
    }
}
