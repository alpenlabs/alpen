//! Runtime validation of complete ASM manifests against stable canonical L1 history.

use strata_asm_common::AsmManifest;
use strata_identifiers::L1Height;
use strata_ol_stf::ExecError;
use strata_primitives::l1::is_l1_reorg_safe;

use crate::{ChainWorkerContext, ManifestPendingReason, WorkerError, WorkerResult};

/// Validates the entire range before the STF can buffer even its first log.
///
/// Reorgs beyond the configured L1 depth are unsupported. Restricting reads to the
/// buried prefix makes the range coherent without trusting cached height mappings.
pub(crate) fn validate_manifests(
    ctx: &impl ChainWorkerContext,
    last_height: L1Height,
    manifests: &[AsmManifest],
) -> WorkerResult<()> {
    // Validate every height first; a later malformed height is not a missing dependency.
    check_manifest_heights(last_height, manifests)?;

    let Some(first) = manifests.first() else {
        return Ok(());
    };

    let tip = ctx
        .canonical_l1_tip_height()
        .map_err(WorkerError::ManifestStorage)?
        .ok_or(WorkerError::ManifestPending {
            height: first.height(),
            reason: ManifestPendingReason::MissingTip,
        })?;
    let depth = ctx.l1_reorg_safe_depth();

    for manifest in manifests {
        let height = manifest.height();
        if !is_l1_reorg_safe(height, tip, depth) {
            return Err(WorkerError::ManifestPending {
                height,
                reason: ManifestPendingReason::NotBuried,
            });
        }

        let canonical = ctx
            .canonical_manifest(height)
            .map_err(WorkerError::ManifestStorage)?
            .ok_or(WorkerError::ManifestPending {
                height,
                reason: ManifestPendingReason::MissingManifest,
            })?;

        if canonical.compute_hash() != manifest.compute_hash() {
            return Err(WorkerError::ManifestContentMismatch { height });
        }
    }

    Ok(())
}

/// Checks that manifest heights continue consecutively after `last_l1_height`.
///
/// Reports the same errors the STF raises when it buffers each manifest, but over the
/// whole range at once and without touching state, so execution order cannot turn a
/// later malformed height into a pending canonical dependency.
fn check_manifest_heights(last_l1_height: L1Height, manifests: &[AsmManifest]) -> WorkerResult<()> {
    let mut expected = last_l1_height;
    for (index, manifest) in manifests.iter().enumerate() {
        expected = expected
            .checked_add(1)
            .ok_or(ExecError::AsmManifestHeightOverflow)?;
        if manifest.height() != expected {
            return Err(ExecError::AsmManifestHeightMismatch {
                expected,
                actual: manifest.height(),
                index,
            }
            .into());
        }
    }
    Ok(())
}
