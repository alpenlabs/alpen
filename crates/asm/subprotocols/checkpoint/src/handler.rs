//! Checkpoint transaction handler.
//!
//! This module handles the processing of individual checkpoint transactions,
//! coordinating verification, state updates, and message forwarding.

use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_common::{AsmLogEntry, MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_logs::CheckpointUpdate;
use strata_asm_proto_checkpoint_txs::extract_signed_checkpoint_from_envelope;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinTxid};

use crate::{
    error::{CheckpointError, CheckpointResult},
    state::CheckpointState,
    verification::{
        construct_checkpoint_claim, validate_epoch_sequence, validate_l1_progression,
        validate_l2_progression, verify_checkpoint_proof, verify_checkpoint_signature,
    },
};

/// Process a checkpoint transaction.
pub(crate) fn handle_checkpoint_tx(
    state: &mut CheckpointState,
    tx: &TxInputRef<'_>,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    // 1. Extract signed checkpoint from envelope
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;

    // 2. Verify signature
    if !verify_checkpoint_signature(&signed_checkpoint, &state.sequencer_cred) {
        return Err(CheckpointError::InvalidSignature);
    }

    let checkpoint = signed_checkpoint.payload();
    let batch_info = checkpoint.batch_info();

    // 3. Validate state transitions
    validate_epoch_sequence(state, batch_info.epoch())?;
    validate_l1_progression(state, batch_info.final_l1_block().height_u64())?;
    validate_l2_progression(state, batch_info.final_l2_block().slot())?;

    // 4. Get manifest hashes from auxiliary data (requested during pre-process phase)
    let prev_l1_height = state.last_checkpoint_l1.height_u64();
    let new_l1_height = batch_info.final_l1_block().height_u64();
    let manifest_hashes = verified_aux_data
        .get_manifest_hashes(prev_l1_height, new_l1_height)
        .map_err(|e| {
            logging::debug!(error = ?e, "Failed to retrieve manifest hashes");
            CheckpointError::MissingManifestHashes
        })?;

    // 5. Construct claim and verify proof (start values from state, end values from payload)
    let claim = construct_checkpoint_claim(state, checkpoint, &manifest_hashes)?;
    verify_checkpoint_proof(&state.checkpoint_predicate, &claim, checkpoint.proof())?;

    // 6. Update state with verified checkpoint
    state.update_with_checkpoint(checkpoint);

    // 7. Forward withdrawal intents to bridge
    forward_withdrawal_intents(checkpoint, relayer);

    // 8. Emit checkpoint update log
    emit_checkpoint_log(tx, checkpoint, relayer);

    Ok(())
}

/// Forward withdrawal intents to the bridge subprotocol.
///
/// Parses the OL logs from the checkpoint sidecar, filters for withdrawal intents
/// from the bridge gateway account, and forwards them to the bridge subprotocol.
fn forward_withdrawal_intents(checkpoint: &CheckpointPayload, relayer: &mut impl MsgRelayer) {
    let Some(logs) = checkpoint.sidecar().parse_ol_logs() else {
        logging::warn!(
            epoch = checkpoint.epoch(),
            "Failed to parse OL logs from checkpoint"
        );
        return;
    };

    let mut withdrawal_count = 0;

    for log in logs
        .iter()
        .filter(|l| l.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
    {
        let Some(withdrawal_data) =
            strata_codec::decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload()).ok()
        else {
            logging::debug!("Failed to decode withdrawal intent log payload");
            continue;
        };

        let Ok(destination) = Descriptor::from_bytes(withdrawal_data.dest()) else {
            logging::debug!("Failed to parse withdrawal destination descriptor");
            continue;
        };

        let withdraw_output = WithdrawOutput::new(destination, withdrawal_data.amt().into());
        let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);
        relayer.relay_msg(&bridge_msg);
        withdrawal_count += 1;
    }

    if withdrawal_count > 0 {
        logging::info!(
            withdrawal_count,
            epoch = checkpoint.epoch(),
            "Forwarded withdrawal intents to bridge"
        );
    }
}

/// Emit a checkpoint update log.
fn emit_checkpoint_log(
    tx: &TxInputRef<'_>,
    checkpoint: &CheckpointPayload,
    relayer: &mut impl MsgRelayer,
) {
    let checkpoint_txid = BitcoinTxid::new(&tx.tx().compute_txid());
    let checkpoint_update = CheckpointUpdate::from_payload(checkpoint, checkpoint_txid);

    match AsmLogEntry::from_log(&checkpoint_update) {
        Ok(log_entry) => {
            relayer.emit_log(log_entry);
            logging::info!(
                txid = %tx.tx().compute_txid(),
                epoch = checkpoint.epoch(),
                "Emitted checkpoint update log"
            );
        }
        Err(err) => {
            logging::error!(error = ?err, "Failed to encode checkpoint update log");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use strata_asm_common::{AsmLogEntry, InterprotoMsg, MsgRelayer};
    use strata_identifiers::CredRule;
    use strata_predicate::PredicateKey;
    use strata_test_utils_asm::checkpoint::{
        CheckpointFixtures, SequencerKeypair, gen_l1_block_commitment,
    };

    use super::*;
    use crate::state::CheckpointConfig;

    /// Mock message relayer for testing.
    #[derive(Default)]
    struct MockMsgRelayer {
        relayed_msgs: Vec<String>,
        emitted_logs: Vec<AsmLogEntry>,
    }

    impl MsgRelayer for MockMsgRelayer {
        fn relay_msg(&mut self, m: &dyn InterprotoMsg) {
            self.relayed_msgs.push(format!("{:?}", m.id()));
        }

        fn emit_log(&mut self, log: AsmLogEntry) {
            self.emitted_logs.push(log);
        }

        fn as_mut_any(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn create_test_config_with_fixtures(fixtures: &CheckpointFixtures) -> CheckpointConfig {
        CheckpointConfig {
            sequencer_cred: CredRule::SchnorrKey(fixtures.sequencer.public_key),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1_block: gen_l1_block_commitment(100),
        }
    }

    #[test]
    fn test_verify_checkpoint_signature_success() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let _state = CheckpointState::new(&config);

        let signed = fixtures.gen_signed_payload();

        // Signature should verify with correct credential
        assert!(verify_checkpoint_signature(&signed, &config.sequencer_cred));
    }

    #[test]
    fn test_verify_checkpoint_signature_wrong_key() {
        let fixtures = CheckpointFixtures::new();
        let signed = fixtures.gen_signed_payload();

        // Wrong keypair should fail verification
        let wrong_keypair = SequencerKeypair::random();
        let wrong_cred = CredRule::SchnorrKey(wrong_keypair.public_key);

        assert!(!verify_checkpoint_signature(&signed, &wrong_cred));
    }

    #[test]
    fn test_verify_checkpoint_signature_unchecked() {
        let fixtures = CheckpointFixtures::new();
        let signed = fixtures.gen_signed_payload();

        // Unchecked credential should always pass
        let unchecked_cred = CredRule::Unchecked;
        assert!(verify_checkpoint_signature(&signed, &unchecked_cred));
    }

    #[test]
    fn test_validate_epoch_invalid_when_skipping() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // Initial state expects epoch 0, so epoch 1 is invalid
        let result = validate_epoch_sequence(&state, 1);
        assert!(result.is_err());

        if let Err(CheckpointError::InvalidEpoch { expected, actual }) = result {
            assert_eq!(expected, 0);
            assert_eq!(actual, 1);
        } else {
            panic!("Expected InvalidEpoch error");
        }
    }

    #[test]
    fn test_validate_epoch_valid_sequential() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let mut state = CheckpointState::new(&config);

        // Epoch 0 is valid initially
        assert!(validate_epoch_sequence(&state, 0).is_ok());

        // Update state with epoch 0
        let payload_0 = fixtures.gen_payload_for_epoch(0);
        state.update_with_checkpoint(&payload_0);

        // Now epoch 1 is valid
        assert!(validate_epoch_sequence(&state, 1).is_ok());

        // But epoch 0 and 2 are invalid
        assert!(validate_epoch_sequence(&state, 0).is_err());
        assert!(validate_epoch_sequence(&state, 2).is_err());
    }

    #[test]
    fn test_validate_l1_height_regression_invalid() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // Genesis at height 100, so height <= 100 is invalid
        let result = validate_l1_progression(&state, 100);
        assert!(result.is_err());

        if let Err(CheckpointError::InvalidL1Height { previous, new }) = result {
            assert_eq!(previous, 100);
            assert_eq!(new, 100);
        } else {
            panic!("Expected InvalidL1Height error");
        }

        // Height below genesis is also invalid
        let result = validate_l1_progression(&state, 50);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_l1_height_progression_valid() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // Any height > 100 is valid
        assert!(validate_l1_progression(&state, 101).is_ok());
        assert!(validate_l1_progression(&state, 200).is_ok());
        assert!(validate_l1_progression(&state, 1000).is_ok());
    }

    #[test]
    fn test_forward_withdrawal_intents_empty_logs() {
        let fixtures = CheckpointFixtures::new();
        let checkpoint = fixtures.gen_payload_for_epoch(0);
        let mut relayer = MockMsgRelayer::default();

        // Forward withdrawal intents - with empty sidecar logs, nothing should be forwarded
        forward_withdrawal_intents(&checkpoint, &mut relayer);

        // No messages should be relayed for empty checkpoint without withdrawal intents
        // (The actual behavior depends on whether the sidecar has logs)
        // This test verifies the function doesn't panic
    }

    #[test]
    fn test_checkpoint_state_update_after_verification() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let mut state = CheckpointState::new(&config);

        // Initial state
        assert!(state.current_epoch().is_none());
        assert_eq!(state.expected_next_epoch(), 0);

        // Apply epoch 0 checkpoint
        let payload_0 = fixtures.gen_payload_for_epoch(0);
        state.update_with_checkpoint(&payload_0);

        // Verify state updated
        assert_eq!(state.current_epoch(), Some(0));
        assert_eq!(state.expected_next_epoch(), 1);

        // Verify last L2 terminal is set
        let terminal = state.last_l2_terminal().expect("Should have terminal");
        assert_eq!(terminal, payload_0.batch_info().final_l2_block());
    }

    #[test]
    fn test_proof_verification_always_accept() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        let payload = fixtures.gen_payload_for_epoch(0);
        let manifest_hashes = vec![];

        // Construct claim
        let claim = construct_checkpoint_claim(&state, &payload, &manifest_hashes).unwrap();

        // With always_accept predicate, any proof should pass
        let result = verify_checkpoint_proof(&config.checkpoint_predicate, &claim, payload.proof());
        assert!(result.is_ok());
    }

    #[test]
    fn test_proof_verification_never_accept() {
        let fixtures = CheckpointFixtures::new();
        let mut config = create_test_config_with_fixtures(&fixtures);
        config.checkpoint_predicate = PredicateKey::never_accept();
        let state = CheckpointState::new(&config);

        let payload = fixtures.gen_payload_for_epoch(0);
        let manifest_hashes = vec![];

        // Construct claim
        let claim = construct_checkpoint_claim(&state, &payload, &manifest_hashes).unwrap();

        // With never_accept predicate, any proof should fail
        let result = verify_checkpoint_proof(&config.checkpoint_predicate, &claim, payload.proof());
        assert!(result.is_err());

        if let Err(CheckpointError::ProofVerification) = result {
            // Expected error
        } else {
            panic!("Expected ProofVerification error");
        }
    }
}
