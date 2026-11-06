//! Test fixtures for the L1 observation and finality that [`crate::FinalizedOLChainProvider`]
//! resolves.

use strata_asm_checkpoint_types::test_utils::create_test_checkpoint_payload;
use strata_asm_checkpoint_types::{CheckpointPayload, CheckpointTip};
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::{EpochCommitment, L1BlockCommitment, L1Height, RBuf32};
use strata_storage::NodeStorage;

/// Returns a checkpoint payload for `commitment` with a sample sidecar and proof.
pub fn build_checkpoint_payload(
    commitment: EpochCommitment,
    l1_height: L1Height,
) -> CheckpointPayload {
    let sample = create_test_checkpoint_payload(commitment.epoch());
    CheckpointPayload::new(
        CheckpointTip::new(
            commitment.epoch(),
            l1_height,
            commitment.to_block_commitment(),
        ),
        sample.sidecar().clone(),
        sample.proof().to_vec(),
    )
    .expect("test: sample sidecar and proof form a valid payload")
}

/// Appends `block` as the next canonical L1 block unless it is already canonical,
/// then records `payload` as `commitment`'s checkpoint observation.
///
/// Returns the corresponding L1 checkpoint for use in client state.
///
/// # Panics
///
/// Panics if the payload tip does not match `commitment`, `block` cannot be
/// appended to canonical L1 history, or storage fails.
pub fn record_l1_observation(
    storage: &NodeStorage,
    commitment: EpochCommitment,
    payload: CheckpointPayload,
    block: L1BlockCommitment,
) -> L1Checkpoint {
    append_canonical_l1_block(storage, block);
    let tip = *payload.new_tip();
    let reference = CheckpointL1Ref::new(block, RBuf32::from([2; 32]), RBuf32::from([3; 32]));
    storage
        .ol_checkpoint()
        .put_checkpoint_l1_observation_blocking(commitment, payload, reference.clone())
        .expect("test: record L1 checkpoint observation");
    L1Checkpoint::new(tip, reference)
}

/// Appends `at` as the next canonical L1 block unless it is already canonical,
/// then persists client state at `at` that finalizes `checkpoint`.
///
/// # Panics
///
/// Panics if `at` cannot be appended to canonical L1 history or storage fails.
pub fn finalize_epoch(storage: &NodeStorage, checkpoint: L1Checkpoint, at: L1BlockCommitment) {
    append_canonical_l1_block(storage, at);
    storage
        .client_state()
        .put_update_blocking(
            &at,
            ClientUpdateOutput::new_state(ClientState::new(
                Some(checkpoint.clone()),
                Some(checkpoint),
            )),
        )
        .expect("test: persist L1 finality");
}

fn append_canonical_l1_block(storage: &NodeStorage, block: L1BlockCommitment) {
    let l1 = storage.l1();
    let canonical = l1
        .get_canonical_blockid_at_height(block.height())
        .expect("test: read canonical L1 block");
    if canonical != Some(*block.blkid()) {
        l1.extend_canonical_chain(block.blkid(), block.height())
            .expect("test: append canonical L1 block");
    }
}
