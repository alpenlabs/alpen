//! The spec of the next epoch after a terminal state.

use strata_identifiers::{L1BlockCommitment, L1BlockId, L1Height};
use strata_ol_chain_types_v1::AsmManifest;
use strata_ol_state_types::{OLSpecId, OLSpecVersions};
use strata_ol_stf_v1::{ExecError, ExecResult, has_checkpoint_predicate_enactment};
use thiserror::Error;

/// Returns the spec the epoch after `parent_versions` runs under, where
/// `parent_versions` are those of the previous epoch's terminal state.
///
/// `parent_last_manifest` is the manifest at the parent state's last L1
/// height, or `None` if the parent state has processed no manifest since
/// genesis. [`select_next_epoch_spec`] looks it up and checks it.
///
/// # From V0 to V1
///
/// A V0 state commits no staged spec, so the switch to V1 is read from the
/// manifests instead: the V0 epoch whose last manifest carries a checkpoint
/// predicate enactment is the last V0 epoch, and the next epoch runs V1, whose
/// epoch-initial processing wraps the state. The parent's last manifest shows
/// whether the parent epoch was that epoch:
///
/// - The ASM accepts no checkpoint whose L1 range crosses an enactment height, so an epoch that
///   processes an enactment ends with it.
/// - A parent epoch that processed no manifests keeps the last L1 height of an earlier epoch. If
///   that height carried the enactment, the epoch after it already ran V1 and the parent state is
///   no longer V0.
///
/// # Invariant
///
/// This relies on V0 history holding no `OlStfVk` enactment except the switch
/// to V1. The manifests are the ones this node's ASM rebuilt from L1, and that
/// ASM emits `CheckpointPredicateEnacted` for every `OlStfVk` update, including
/// updates from before 0.3.x ran an ASM that emits the log. An enactment
/// anywhere else would select V1 too early. No V0 epoch follows the switch:
/// the ASM verifies the L1 heights after it under the V1 predicate.
///
/// A wrong selection fails rather than diverges: the root form of the epoch
/// replayed under the wrong rules would not match, so the reconstructed
/// terminal header would not reproduce the block ID the ASM verified.
///
/// # Errors
///
/// Returns an error if the manifest carries a malformed or duplicated
/// checkpoint predicate enactment.
pub fn next_epoch_spec(
    parent_versions: OLSpecVersions,
    parent_last_manifest: Option<&AsmManifest>,
) -> ExecResult<OLSpecId> {
    match parent_versions.cur_spec() {
        OLSpecId::V0 => {
            let enacted = match parent_last_manifest {
                Some(manifest) => has_checkpoint_predicate_enactment(manifest)?,
                None => false,
            };
            Ok(if enacted { OLSpecId::V1 } else { OLSpecId::V0 })
        }
        // The staged spec is not followed until spec advancement lands; until
        // then nothing stages a spec after V1.
        // TODO(STR-4086): run the spec the parent state stages.
        OLSpecId::V1 => Ok(OLSpecId::V1),
    }
}

/// Errors returned by [`select_next_epoch_spec`].
#[derive(Debug, Error)]
pub enum EpochSpecSelectionError<E> {
    /// Looking up the stored L1 manifest failed.
    #[error("look up the L1 manifest at height {height}: {source}")]
    ManifestLookup {
        height: L1Height,
        #[source]
        source: E,
    },

    /// No L1 manifest is stored at the height the parent state processed last.
    #[error("missing the L1 manifest at height {height} the parent state processed last")]
    MissingLastManifest { height: L1Height },

    /// The stored L1 manifest at the parent state's last L1 height is for
    /// another L1 block than the one the parent state processed.
    #[error(
        "stored L1 manifest at height {height} is for block {found}, but the parent state processed {expected}"
    )]
    LastManifestMismatch {
        height: L1Height,
        expected: L1BlockId,
        found: L1BlockId,
    },

    /// The manifest carries a malformed or duplicated checkpoint predicate
    /// enactment.
    #[error("select the next epoch's spec: {0}")]
    Exec(#[from] ExecError),
}

/// Selects the spec of the epoch after the parent state, the previous epoch's
/// terminal state, with [`next_epoch_spec`].
///
/// `parent_versions` and `parent_last_l1` are the parent state's spec versions
/// and last L1 block. For a V0 parent this reads the manifest the parent
/// processed last with `manifest_at`, which returns the stored manifest at an
/// L1 height, or `None` if none is stored. Genesis takes `genesis_l1_block`,
/// the L1 anchor, as its last L1 block without processing a manifest for it,
/// and the ASM stores none, so a parent still at the anchor has processed no
/// manifest. Any later manifest must be stored, and must be for the L1 block
/// the parent recorded, so a reorged manifest is never read as the parent's.
///
/// # Errors
///
/// Returns an error if the lookup fails, the manifest is missing or for
/// another L1 block, or [`next_epoch_spec`] fails.
pub fn select_next_epoch_spec<E>(
    parent_versions: OLSpecVersions,
    parent_last_l1: L1BlockCommitment,
    genesis_l1_block: L1BlockCommitment,
    manifest_at: impl FnOnce(L1Height) -> Result<Option<AsmManifest>, E>,
) -> Result<OLSpecId, EpochSpecSelectionError<E>> {
    let needs_last_manifest =
        parent_versions.cur_spec() == OLSpecId::V0 && parent_last_l1 != genesis_l1_block;
    let last_manifest = if needs_last_manifest {
        let height = parent_last_l1.height();
        let manifest = manifest_at(height)
            .map_err(|source| EpochSpecSelectionError::ManifestLookup { height, source })?
            .ok_or(EpochSpecSelectionError::MissingLastManifest { height })?;
        if manifest.blkid() != parent_last_l1.blkid() {
            return Err(EpochSpecSelectionError::LastManifestMismatch {
                height,
                expected: *parent_last_l1.blkid(),
                found: *manifest.blkid(),
            });
        }
        Some(manifest)
    } else {
        None
    };
    Ok(next_epoch_spec(parent_versions, last_manifest.as_ref())?)
}
