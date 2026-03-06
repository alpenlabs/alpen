//! Test utilities and proptest strategies for checkpoint types.

use proptest::prelude::*;
use strata_identifiers::{
    AccountSerial, Buf32, OLBlockCommitment,
    test_utils::{
        buf32_strategy, epoch_strategy, fixed_bytes_32_strategy, ol_block_commitment_strategy,
        ol_block_id_strategy,
    },
};

use crate::{
    CheckpointClaim, CheckpointPayload, CheckpointSidecar, CheckpointTip, L2BlockRange,
    MAX_LOG_PAYLOAD_BYTES, TerminalHeaderComplement,
};

/// Creates a minimal [`CheckpointPayload`] for the given epoch using validated constructors.
pub fn create_test_checkpoint_payload(epoch: u32) -> CheckpointPayload {
    let tip = CheckpointTip::new(epoch, 200, OLBlockCommitment::new(1, Buf32::zero().into()));
    let sidecar = CheckpointSidecar::new(
        vec![2; 100],
        vec![],
        TerminalHeaderComplement::new(0, Buf32::zero().into(), Buf32::zero(), Buf32::zero()),
    )
    .expect("test sidecar is within size limits");

    CheckpointPayload::new(tip, sidecar, vec![0]).expect("test payload is within size limits")
}

/// Strategy for generating random [`CheckpointTip`] values.
pub fn checkpoint_tip_strategy() -> impl Strategy<Value = CheckpointTip> {
    (
        epoch_strategy(),
        any::<u32>(),
        ol_block_commitment_strategy(),
    )
        .prop_map(|(epoch, l1_height, l2_commitment)| {
            CheckpointTip::new(epoch, l1_height, l2_commitment)
        })
}

/// Strategy for generating random state diff bytes of varying sizes.
fn state_diff_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..1024)
}

/// Strategy for generating random proof bytes of varying sizes.
fn proof_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..512)
}

/// Strategy for generating random OL logs of varying sizes.
fn ol_logs_strategy() -> impl Strategy<Value = Vec<crate::OLLog>> {
    prop::collection::vec(
        (
            any::<u32>().prop_map(AccountSerial::from),
            prop::collection::vec(any::<u8>(), 0..=MAX_LOG_PAYLOAD_BYTES),
        )
            .prop_map(|(account_serial, payload)| crate::OLLog::new(account_serial, payload)),
        0..10,
    )
}

/// Strategy for generating random [`crate::TerminalHeaderComplement`] values.
fn terminal_header_complement_strategy() -> impl Strategy<Value = crate::TerminalHeaderComplement> {
    (
        any::<u64>(),
        ol_block_id_strategy(),
        buf32_strategy(),
        buf32_strategy(),
    )
        .prop_map(|(timestamp, parent_blkid, body_root, logs_root)| {
            crate::TerminalHeaderComplement::new(timestamp, parent_blkid, body_root, logs_root)
        })
}

/// Strategy for generating random [`CheckpointSidecar`] values.
pub fn checkpoint_sidecar_strategy() -> impl Strategy<Value = CheckpointSidecar> {
    (
        state_diff_strategy(),
        ol_logs_strategy(),
        terminal_header_complement_strategy(),
    )
        .prop_map(|(state_diff, ol_logs, terminal_header_complement)| {
            CheckpointSidecar::new(state_diff, ol_logs, terminal_header_complement)
                .expect("valid sidecar")
        })
}

/// Strategy for generating random [`CheckpointPayload`] values.
pub fn checkpoint_payload_strategy() -> impl Strategy<Value = CheckpointPayload> {
    (
        checkpoint_tip_strategy(),
        checkpoint_sidecar_strategy(),
        proof_strategy(),
    )
        .prop_map(|(tip, sidecar, proof)| {
            CheckpointPayload::new(tip, sidecar, proof).expect("valid payload")
        })
}

/// Strategy for generating random [`L2BlockRange`] values.
pub fn l2_block_range_strategy() -> impl Strategy<Value = L2BlockRange> {
    (
        ol_block_commitment_strategy(),
        ol_block_commitment_strategy(),
    )
        .prop_map(|(start, end)| L2BlockRange::new(start, end))
}

/// Strategy for generating random [`CheckpointClaim`] values.
pub fn checkpoint_claim_strategy() -> impl Strategy<Value = CheckpointClaim> {
    (
        epoch_strategy(),
        l2_block_range_strategy(),
        fixed_bytes_32_strategy(),
        fixed_bytes_32_strategy(),
        fixed_bytes_32_strategy(),
        fixed_bytes_32_strategy(),
    )
        .prop_map(
            |(
                epoch,
                l2_range,
                asm_manifests_hash,
                state_diff_hash,
                ol_logs_hash,
                terminal_header_complement_hash,
            )| {
                CheckpointClaim::new(
                    epoch,
                    l2_range,
                    asm_manifests_hash,
                    state_diff_hash,
                    ol_logs_hash,
                    terminal_header_complement_hash,
                )
            },
        )
}
