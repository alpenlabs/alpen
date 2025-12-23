//! Utility functions for the checkpoint subprotocol.

use strata_asm_common::VerifiedAuxData;
use strata_asm_manifest_types::Hash32;
use strata_checkpoint_types_ssz::BatchInfo;
use strata_identifiers::{Buf32, hash};

use crate::{error::CheckpointResult, state::CheckpointState};

/// Get the L1 height range for manifest hash retrieval.
///
/// Returns (start_height, end_height) where:
/// - start_height: One past the last L1 block covered by the previous checkpoint
/// - end_height: The end L1 block of the current checkpoint's range
///
/// TODO: Verify start/end height calculation aligns with L1BlockRange semantics
/// (inclusive vs exclusive bounds).
pub(crate) fn get_manifest_hash_range(
    state: &CheckpointState,
    batch_info: &BatchInfo,
) -> (u64, u64) {
    // L1BlockRange is inclusive on both ends; start is the previous checkpoint's final L1 block.
    // We only need new manifests, so begin one past the last covered height.
    let start_height = state.last_covered_l1_height() as u64 + 1;
    let end_height = batch_info.l1_range.end.height as u64;
    (start_height, end_height)
}

/// Retrieve manifest hashes from auxiliary data for the checkpoint's L1 range.
pub(crate) fn get_manifest_hashes(
    state: &CheckpointState,
    batch_info: &BatchInfo,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointResult<Vec<Hash32>> {
    let (start_height, end_height) = get_manifest_hash_range(state, batch_info);
    Ok(verified_aux_data.get_manifest_hashes(start_height, end_height)?)
}

/// Compute a commitment over manifest hashes.
///
/// This creates a single hash over all concatenated manifest hashes to commit
/// to the input messages from L1 in the specified block range.
pub(crate) fn compute_manifest_hashes_commitment(manifest_hashes: &[Hash32]) -> Buf32 {
    if manifest_hashes.is_empty() {
        return Buf32::zero();
    }

    // Concatenate all hashes and compute a single hash
    let mut data = Vec::with_capacity(manifest_hashes.len() * 32);
    for h in manifest_hashes {
        data.extend_from_slice(h.as_ref());
    }

    hash::raw(&data)
}
