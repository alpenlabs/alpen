//! Runtime validation of complete ASM manifests against stable canonical L1 history.

use strata_asm_common::AsmManifest;
use strata_identifiers::L1Height;
use strata_ol_stf_v1::{ExecError, validate_manifest_heights};

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
    validate_manifest_heights(last_height, manifests)?;

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
    let buried_tip = tip.checked_sub(ctx.l1_reorg_safe_depth().saturating_sub(1));

    for manifest in manifests {
        let height = manifest.height();
        if buried_tip.is_none_or(|tip| height > tip) {
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
            return Err(ExecError::AsmManifestContentMismatch { height }.into());
        }
    }

    Ok(())
}
