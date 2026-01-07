//! Checkpoint transaction handler.
//!
//! This module handles the processing of individual checkpoint transactions,
//! coordinating verification, state updates, and message forwarding.

use ssz::Encode;
use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_common::{AsmLogEntry, MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_logs::CheckpointUpdate;
use strata_asm_proto_checkpoint_txs::extract_signed_checkpoint_from_envelope;
use strata_checkpoint_types_ssz::{BatchInfo, CheckpointClaim, CheckpointPayload};
use strata_codec::decode_buf_exact;
use strata_identifiers::Buf32;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinTxid};

use crate::{
    error::{CheckpointError, CheckpointResult},
    state::CheckpointState,
    utils::{compute_manifest_hashes_commitment, get_manifest_hashes},
};

/// Process a checkpoint transaction.
///
/// Steps:
/// 1. Extract signed checkpoint from envelope
/// 2. Verify signature using predicate framework.
/// 3. Validate start values match expected state
/// 4. Validate state transitions (epoch, L1/L2 progression)
/// 5. Get manifest hashes from auxiliary data
/// 6. Construct claim and verify proof
/// 7. Update state with verified checkpoint
/// 8. Forward withdrawal intents to bridge
/// 9. Emit checkpoint update log
pub(crate) fn handle_checkpoint_tx(
    state: &mut CheckpointState,
    tx: &TxInputRef<'_>,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    // 1. Extract signed checkpoint from envelope
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;
    let checkpoint_payload = &signed_checkpoint.inner;

    // 2. Verify signature using predicate framework.
    // The predicate verifier expects raw payload bytes (not pre-hashed) because
    // BIP-340 Schnorr verification hashes the message internally using tagged hashing.
    let payload_bytes = checkpoint_payload.as_ssz_bytes();
    state
        .sequencer_predicate()
        .verify_claim_witness(&payload_bytes, signed_checkpoint.signature.as_ref())
        .map_err(|_| CheckpointError::InvalidSignature)?;
    let batch_info = &checkpoint_payload.commitment.batch_info;

    // 3. Validate start values match expected state
    validate_start_values(state, batch_info)?;

    // 4. Validate state transitions (epoch, L1/L2 progression)
    validate_state_transitions(state, batch_info)?;

    // 5. Get manifest hashes from auxiliary data
    let manifest_hashes = get_manifest_hashes(state, batch_info, verified_aux_data)?;

    // 6. Construct claim and verify proof
    let pre_state_root = state.pre_state_root();
    let input_msgs_commitment = compute_manifest_hashes_commitment(&manifest_hashes)?;
    let claim =
        CheckpointClaim::from_payload(checkpoint_payload, pre_state_root, input_msgs_commitment);

    state
        .checkpoint_predicate()
        .verify_claim_witness(&claim.to_bytes(), &checkpoint_payload.proof)?;

    // 7. Update state with verified checkpoint
    state.update_with_checkpoint(checkpoint_payload);

    // 8. Forward withdrawal intents to bridge
    forward_withdrawal_intents(checkpoint_payload, relayer);

    // 9. Emit checkpoint update log
    let checkpoint_txid = BitcoinTxid::from(tx.tx().compute_txid());
    emit_checkpoint_log(checkpoint_payload, pre_state_root, checkpoint_txid, relayer)?;

    Ok(())
}

/// Validate that checkpoint start values match expected state.
///
/// The L1 range uses `[start, end]` semantics where `start` is the first L1 block
/// covered by this checkpoint (one past the previous checkpoint's end).
fn validate_start_values(state: &CheckpointState, batch_info: &BatchInfo) -> CheckpointResult<()> {
    // L1 range start height must be one past the last covered L1 height.
    // The blkid is validated indirectly via manifest hash verification in the ZK proof.
    let last_l1_height = state.last_covered_l1_height();
    let expected_start_height = last_l1_height + 1;
    let l1_start = batch_info.l1_range.start;
    if l1_start.height_u64() as u32 != expected_start_height {
        return Err(CheckpointError::InvalidL1Start {
            expected_height: expected_start_height,
            expected_blkid: *state.last_covered_l1().blkid(), // For error context only
            new_height: l1_start.height_u64() as u32,
            new_blkid: *l1_start.blkid(),
        });
    }

    // L2 range start slot must be previous terminal + 1 (or 1 for first checkpoint).
    // Only validate slot continuity, not blkid (similar to L1 height-only check).
    let last_l2_slot = state.last_l2_terminal().map(|t| t.slot()).unwrap_or(0);
    let expected_l2_start_slot = last_l2_slot + 1;
    let l2_start = batch_info.l2_range.start;
    if l2_start.slot() != expected_l2_start_slot {
        return Err(CheckpointError::InvalidL2Start {
            expected_slot: expected_l2_start_slot,
            expected_blkid: Buf32::zero().into(), // Not validated, for error context only
            new_slot: l2_start.slot(),
            new_blkid: *l2_start.blkid(),
        });
    }

    Ok(())
}

/// Validate state transitions: epoch sequence, L1 height progression, L2 slot progression.
fn validate_state_transitions(
    state: &CheckpointState,
    batch_info: &BatchInfo,
) -> CheckpointResult<()> {
    // Epoch must be sequential
    let expected_epoch = state.expected_next_epoch();
    if batch_info.epoch != expected_epoch {
        return Err(CheckpointError::InvalidEpoch {
            expected: expected_epoch,
            actual: batch_info.epoch,
        });
    }

    // L1 end height must progress beyond last covered L1
    let last_l1_height = state.last_covered_l1_height();
    let l1_end = batch_info.l1_range.end.height_u64() as u32;
    if l1_end <= last_l1_height {
        return Err(CheckpointError::InvalidL1Progression {
            previous: last_l1_height,
            new: l1_end,
        });
    }

    // L2 end slot must progress beyond last terminal (if any)
    if let Some(last_l2_slot) = state.last_l2_terminal_slot() {
        let l2_end = batch_info.l2_range.end.slot();
        if l2_end <= last_l2_slot {
            return Err(CheckpointError::InvalidL2Progression {
                previous: last_l2_slot,
                new: l2_end,
            });
        }
    }

    Ok(())
}

/// Forward withdrawal intents to the bridge subprotocol.
///
/// Parses the OL logs from the checkpoint sidecar, filters for withdrawal intents
/// from the bridge gateway account, and forwards them to the bridge subprotocol.
fn forward_withdrawal_intents(checkpoint: &CheckpointPayload, relayer: &mut impl MsgRelayer) {
    let Some(logs) = checkpoint.sidecar.parse_ol_logs() else {
        logging::warn!(
            epoch = checkpoint.epoch(),
            "Failed to parse OL logs from checkpoint"
        );
        return;
    };

    for log in logs
        .iter()
        .filter(|l| l.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
    {
        // Decode withdrawal intent log data
        // Skip malformed logs with a warning
        let Ok(withdrawal_data) = decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload())
        else {
            logging::warn!("Failed to decode withdrawal intent log payload");
            continue;
        };

        // Parse destination descriptor
        // Skip malformed descriptors with a warning
        let Ok(destination) = Descriptor::from_bytes(withdrawal_data.dest()) else {
            logging::warn!("Failed to parse withdrawal destination descriptor");
            continue;
        };

        let withdraw_output = WithdrawOutput::new(destination, withdrawal_data.amt().into());
        let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);
        relayer.relay_msg(&bridge_msg);
    }
}

/// Emit a checkpoint update log entry.
fn emit_checkpoint_log(
    checkpoint: &CheckpointPayload,
    pre_state_root: Buf32,
    checkpoint_txid: BitcoinTxid,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    let checkpoint_update =
        CheckpointUpdate::from_payload(checkpoint, pre_state_root, checkpoint_txid);
    let log_entry = AsmLogEntry::from_log(&checkpoint_update)?;
    relayer.emit_log(log_entry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use strata_asm_bridge_msgs::BridgeIncomingMsg;
    use strata_asm_common::{AsmLogEntry, InterprotoMsg, MsgRelayer, TxInputRef, VerifiedAuxData};
    use strata_asm_logs::CheckpointUpdate;
    use strata_asm_proto_checkpoint_txs::test_utils::{
        CheckpointGenerator, SequencerKeypair, build_l1_payload, verified_aux_data_for_heights,
    };
    use strata_btcio::test_utils::create_checkpoint_envelope_tx;
    use strata_checkpoint_types_ssz::{L1BlockCommitment, SignedCheckpointPayload};
    use strata_codec::encode_to_vec;
    use strata_identifiers::{Buf32, L1BlockId};
    use strata_l1_txfmt::{MagicBytes, ParseConfig};
    use strata_ol_chain_types_new::{OLLog, SimpleWithdrawalIntentLogData};
    use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;
    use strata_predicate::PredicateKey;
    use strata_primitives::bitcoin_bosd::Descriptor;

    use super::*;
    use crate::CheckpointConfig;

    const TEST_MAGIC_BYTES: MagicBytes = MagicBytes::new(*b"ALPN");
    const TEST_ADDR: &str = "bcrt1q6u6qyya3sryhh42lahtnz2m7zuufe7dlt8j0j5";

    struct TestRelayer {
        logs: Vec<AsmLogEntry>,
        bridge_msgs: Vec<BridgeIncomingMsg>,
    }

    impl TestRelayer {
        fn new() -> Self {
            Self {
                logs: Vec::new(),
                bridge_msgs: Vec::new(),
            }
        }
    }

    impl MsgRelayer for TestRelayer {
        fn relay_msg(&mut self, m: &dyn InterprotoMsg) {
            if let Some(msg) = m.as_dyn_any().downcast_ref::<BridgeIncomingMsg>() {
                self.bridge_msgs.push(msg.clone());
            }
        }

        fn emit_log(&mut self, log: AsmLogEntry) {
            self.logs.push(log);
        }

        fn as_mut_any(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn genesis_l1() -> strata_checkpoint_types_ssz::L1BlockCommitment {
        strata_checkpoint_types_ssz::L1BlockCommitment::from_height_u64(
            0,
            L1BlockId::from(Buf32::zero()),
        )
        .expect("valid genesis height")
    }

    fn verified_aux_for(state: &CheckpointState, batch_info: &BatchInfo) -> VerifiedAuxData {
        let start_height = state.last_covered_l1_height() as u64 + 1;
        let end_height = batch_info.l1_range.end.height_u64();
        let (verified_aux, _compact_mmr) = verified_aux_data_for_heights(start_height, end_height);
        verified_aux
    }

    #[test]
    fn test_valid_first_checkpoint() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let payload = generator.gen_payload(1, 1, vec![]);
        let signature = keypair.sign(&payload);
        let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer).unwrap();

        let summary = state
            .verified_epoch_summary()
            .expect("epoch summary should be set");
        assert_eq!(summary.epoch(), 0);
        assert_eq!(state.last_covered_l1_height(), 1);
        assert!(relayer.bridge_msgs.is_empty());

        let checkpoint_log: CheckpointUpdate = relayer
            .logs
            .iter()
            .find_map(|l| l.try_into_log().ok())
            .expect("checkpoint log emitted");
        assert_eq!(checkpoint_log.epoch_commitment().epoch(), 0);
        assert_eq!(checkpoint_log.batch_info().epoch, 0);
        assert_eq!(checkpoint_log.batch_info().l1_range.end.height_u64(), 1);
        assert_eq!(
            checkpoint_log.transition().post_state_root,
            payload.commitment.post_state_root
        );
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let signer = SequencerKeypair::random();
        let wrong_signer = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let payload = generator.gen_payload(1, 1, vec![]);
        let signature = wrong_signer.sign(&payload);
        let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: signer.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        let err = handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer)
            .expect_err("signature must be rejected");
        assert!(matches!(err, CheckpointError::InvalidSignature));
        assert!(state.verified_epoch_summary().is_none());
        assert!(relayer.logs.is_empty());
    }

    #[test]
    fn test_invalid_epoch_sequence() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let mut payload = generator.gen_payload(1, 1, vec![]);
        payload.commitment.batch_info.epoch = 1; // state expects 0
        let signature = keypair.sign(&payload);
        let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        let err = handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer)
            .expect_err("unexpected epoch should be rejected");
        assert!(matches!(err, CheckpointError::InvalidEpoch { .. }));
        assert!(state.verified_epoch_summary().is_none());
        assert!(relayer.logs.is_empty());
    }

    #[test]
    fn test_invalid_l1_start_rejected() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let mut payload = generator.gen_payload(1, 1, vec![]);

        // Tamper the L1 start height to mismatch expected (genesis_height + 1 = 1).
        // Use height 999 to ensure it doesn't match the expected value.
        payload.commitment.batch_info.l1_range.start = L1BlockCommitment::from_height_u64(
            999,
            *payload.commitment.batch_info.l1_range.start.blkid(),
        )
        .expect("valid L1 height");

        let signature = keypair.sign(&payload);
        let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        let err = handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer)
            .expect_err("invalid start should be rejected");
        assert!(matches!(err, CheckpointError::InvalidL1Start { .. }));
        assert!(state.verified_epoch_summary().is_none());
        assert!(relayer.logs.is_empty());
    }

    #[test]
    fn test_withdrawal_intent_forwarding() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);

        let descriptor = Descriptor::new_p2wpkh(&[7u8; 20]);
        let log_payload =
            SimpleWithdrawalIntentLogData::new(42, descriptor.to_bytes().to_vec()).unwrap();
        let encoded_payload = encode_to_vec(&log_payload).unwrap();
        let ol_log = OLLog::new(BRIDGE_GATEWAY_ACCT_SERIAL, encoded_payload);

        let payload = generator.gen_payload(1, 1, vec![ol_log]);
        let signature = keypair.sign(&payload);
        let signed_checkpoint =
            strata_checkpoint_types_ssz::SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer).unwrap();

        assert_eq!(relayer.bridge_msgs.len(), 1);
        let BridgeIncomingMsg::DispatchWithdrawal(output) = relayer.bridge_msgs.first().unwrap();
        assert_eq!(output.amt.to_sat(), 42);
        assert_eq!(output.destination.to_bytes(), descriptor.to_bytes());

        let checkpoint_log: CheckpointUpdate = relayer
            .logs
            .iter()
            .find_map(|l| l.try_into_log().ok())
            .expect("checkpoint log emitted");
        assert_eq!(checkpoint_log.epoch_commitment().epoch(), 0);
        assert_eq!(checkpoint_log.batch_info().l1_range.end.height_u64(), 1);
        assert_eq!(
            checkpoint_log.transition().post_state_root,
            payload.commitment.post_state_root
        );
    }

    #[test]
    fn test_multiple_checkpoints_in_sequence() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });

        // Process 3 checkpoints in sequence: epoch 0, 1, 2
        for expected_epoch in 0..3 {
            let payload = generator.gen_payload(1, 1, vec![]);
            let signature = keypair.sign(&payload);
            let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
            let l1_payload = build_l1_payload(&signed_checkpoint);
            let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
            let tag = ParseConfig::new(TEST_MAGIC_BYTES)
                .try_parse_tx(&tx)
                .expect("tag data");
            let tx_ref = TxInputRef::new(&tx, tag);
            let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
            let mut relayer = TestRelayer::new();

            handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer).unwrap();

            let summary = state
                .verified_epoch_summary()
                .expect("epoch summary should be set");
            assert_eq!(summary.epoch(), expected_epoch);

            // Advance generator state for next iteration
            generator.advance(&payload);
        }

        // Verify final state
        let summary = state.verified_epoch_summary().expect("should have summary");
        assert_eq!(summary.epoch(), 2);
        assert_eq!(state.expected_next_epoch(), 3);
    }

    #[test]
    fn test_invalid_l2_start_rejected() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let mut payload = generator.gen_payload(1, 1, vec![]);

        // Tamper the L2 start slot to mismatch expected (should be 1 for first checkpoint)
        payload.commitment.batch_info.l2_range.start = strata_identifiers::OLBlockCommitment::new(
            999,
            *payload.commitment.batch_info.l2_range.start.blkid(),
        );

        let signature = keypair.sign(&payload);
        let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        let err = handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer)
            .expect_err("invalid L2 start should be rejected");
        assert!(matches!(err, CheckpointError::InvalidL2Start { .. }));
        assert!(state.verified_epoch_summary().is_none());
    }

    #[test]
    fn test_invalid_l2_progression_rejected() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });

        // First, process a valid checkpoint to establish L2 terminal
        let payload1 = generator.gen_payload(1, 10, vec![]); // L2 slots 1-10
        let signature1 = keypair.sign(&payload1);
        let signed1 = SignedCheckpointPayload::new(payload1.clone(), signature1);
        let l1_payload1 = build_l1_payload(&signed1);
        let tx1 = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload1);
        let tag1 = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx1)
            .expect("tag data");
        let tx_ref1 = TxInputRef::new(&tx1, tag1);
        let verified_aux1 = verified_aux_for(&state, &payload1.commitment.batch_info);
        let mut relayer1 = TestRelayer::new();
        handle_checkpoint_tx(&mut state, &tx_ref1, &verified_aux1, &mut relayer1).unwrap();
        generator.advance(&payload1);

        // Now create second checkpoint with L2 end that doesn't progress
        let mut payload2 = generator.gen_payload(1, 5, vec![]); // Would be L2 slots 11-15
        // Tamper: set L2 end slot to be <= previous terminal (10)
        payload2.commitment.batch_info.l2_range.end = strata_identifiers::OLBlockCommitment::new(
            5,
            *payload2.commitment.batch_info.l2_range.end.blkid(),
        );

        let signature2 = keypair.sign(&payload2);
        let signed2 = SignedCheckpointPayload::new(payload2.clone(), signature2);
        let l1_payload2 = build_l1_payload(&signed2);
        let tx2 = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload2);
        let tag2 = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx2)
            .expect("tag data");
        let tx_ref2 = TxInputRef::new(&tx2, tag2);
        let verified_aux2 = verified_aux_for(&state, &payload2.commitment.batch_info);
        let mut relayer2 = TestRelayer::new();

        let err = handle_checkpoint_tx(&mut state, &tx_ref2, &verified_aux2, &mut relayer2)
            .expect_err("non-progressing L2 should be rejected");
        assert!(matches!(err, CheckpointError::InvalidL2Progression { .. }));
    }

    #[test]
    fn test_malformed_withdrawal_intent_skipped() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);

        // Create a malformed OL log with garbage payload
        let malformed_log = OLLog::new(BRIDGE_GATEWAY_ACCT_SERIAL, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        // Also include a valid withdrawal
        let descriptor = Descriptor::new_p2wpkh(&[7u8; 20]);
        let valid_log_payload =
            SimpleWithdrawalIntentLogData::new(100, descriptor.to_bytes().to_vec()).unwrap();
        let valid_encoded = encode_to_vec(&valid_log_payload).unwrap();
        let valid_log = OLLog::new(BRIDGE_GATEWAY_ACCT_SERIAL, valid_encoded);

        let payload = generator.gen_payload(1, 1, vec![malformed_log, valid_log]);
        let signature = keypair.sign(&payload);
        let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        // Should succeed - malformed log is skipped, valid one is forwarded
        handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer).unwrap();

        // Only the valid withdrawal should be forwarded
        assert_eq!(relayer.bridge_msgs.len(), 1);
        let BridgeIncomingMsg::DispatchWithdrawal(output) = relayer.bridge_msgs.first().unwrap();
        assert_eq!(output.amt.to_sat(), 100);
    }

    #[test]
    fn test_proof_verification_failure() {
        let keypair = SequencerKeypair::random();
        let genesis_l1 = genesis_l1();
        let mut generator = CheckpointGenerator::new(genesis_l1);
        let payload = generator.gen_payload(1, 1, vec![]);
        let signature = keypair.sign(&payload);
        let signed_checkpoint = SignedCheckpointPayload::new(payload.clone(), signature);
        let l1_payload = build_l1_payload(&signed_checkpoint);

        // Use always_reject predicate for checkpoint proof verification
        let mut state = CheckpointState::new(&CheckpointConfig {
            sequencer_predicate: keypair.sequencer_predicate(),
            checkpoint_predicate: PredicateKey::never_accept(),
            genesis_l1,
            genesis_ol_state_root: Buf32::zero(),
        });
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let tag = ParseConfig::new(TEST_MAGIC_BYTES)
            .try_parse_tx(&tx)
            .expect("tag data");
        let tx_ref = TxInputRef::new(&tx, tag);
        let verified_aux = verified_aux_for(&state, &payload.commitment.batch_info);
        let mut relayer = TestRelayer::new();

        let err = handle_checkpoint_tx(&mut state, &tx_ref, &verified_aux, &mut relayer)
            .expect_err("proof verification should fail");
        assert!(matches!(err, CheckpointError::ProofVerification(_)));
        assert!(state.verified_epoch_summary().is_none());
        assert!(relayer.logs.is_empty());
    }
}
