//! The spec each assembled block runs under.

use strata_checkpoint_types::prev_epoch_last_l1_async;
use strata_identifiers::{EpochCommitment, L1BlockCommitment, OLBlockCommitment};
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_ol_state_types::{IStateAccessor, OLSpecVersions};
use strata_ol_stf::{EpochL1Range, EpochSpecSelectionError, OLSpecId, select_next_epoch_spec};

use crate::{BlockAssemblyAnchorContext, BlockAssemblyError, BlockAssemblyResult};

/// Returns the spec of the block after `parent`, whose header is
/// `parent_header` and whose state is `parent_state`.
///
/// A block after an epoch's terminal block starts the next epoch, whose spec
/// [`select_spec_after_terminal`] selects. Any other block continues its
/// parent's epoch and runs the spec the parent state was produced under:
/// every rule set leaves the state's current spec at its own identifier from
/// the first block of its epoch on. So a terminal block, drain included, runs
/// under the spec of the epoch it ends, as block verification does.
///
/// # Errors
///
/// Returns [`BlockAssemblyError::UpgradeRequired`] if the block runs a spec
/// this binary does not implement, and an error if the parent epoch's
/// summaries or last manifest are missing or inconsistent.
pub(crate) async fn select_block_spec<C: BlockAssemblyAnchorContext>(
    ctx: &C,
    parent: OLBlockCommitment,
    parent_header: &OLBlockHeaderV1,
    parent_state: &impl IStateAccessor,
) -> BlockAssemblyResult<OLSpecId> {
    let parent_versions = parent_state.spec_versions();
    if !parent_header.is_terminal() {
        return Ok(parent_versions.cur_spec());
    }
    let parent_epoch = EpochCommitment::from_terminal(parent_header.epoch(), parent);
    let parent_last_l1 =
        L1BlockCommitment::new(parent_state.last_l1_height(), *parent_state.last_l1_blkid());
    select_spec_after_terminal(ctx, parent_epoch, parent_versions, parent_last_l1).await
}

/// Selects the spec of the epoch after `parent`, whose terminal state has
/// `parent_versions` and last L1 block `parent_last_l1`, with
/// [`select_next_epoch_spec`].
///
/// The parent epoch's L1 range starts at the last L1 block of the epoch its
/// own summary links to, so it follows the parent's chain rather than the
/// canonical one. The genesis epoch starts at the L1 anchor.
///
/// # Errors
///
/// Returns [`BlockAssemblyError::UpgradeRequired`] if the epoch runs a spec
/// this binary does not implement, and an error if the parent epoch's
/// summaries or last manifest are missing or inconsistent.
pub(crate) async fn select_spec_after_terminal<C: BlockAssemblyAnchorContext>(
    ctx: &C,
    parent: EpochCommitment,
    parent_versions: OLSpecVersions,
    parent_last_l1: L1BlockCommitment,
) -> BlockAssemblyResult<OLSpecId> {
    let prev_last = prev_epoch_last_l1_async(parent, ctx.genesis_l1_block(), |epoch| async move {
        ctx.fetch_epoch_summary(epoch)
            .await?
            .ok_or(BlockAssemblyError::EpochSummaryNotFound(epoch))
    })
    .await?;
    let parent_l1_range = EpochL1Range::new(prev_last, parent_last_l1)?;

    // The lookup is async and selection is not, so fetch the one manifest
    // selection reads first.
    let last_manifest = match parent_l1_range.last_processed() {
        Some(block) => ctx.fetch_l1_manifest(block).await?,
        None => None,
    };
    select_next_epoch_spec(parent_versions, parent_l1_range, |_| {
        Ok::<_, BlockAssemblyError>(last_manifest)
    })
    .map_err(|err| match err {
        EpochSpecSelectionError::ManifestLookup { source, .. } => source,
        EpochSpecSelectionError::MissingLastManifest { block } => {
            BlockAssemblyError::MissingLastManifest { block }
        }
        EpochSpecSelectionError::LastManifestMismatch { expected, found } => {
            BlockAssemblyError::LastManifestMismatch { expected, found }
        }
        EpochSpecSelectionError::Exec(source) => BlockAssemblyError::BlockConstruction(source),
        EpochSpecSelectionError::UpgradeRequired(upgrade) => {
            BlockAssemblyError::UpgradeRequired(upgrade)
        }
    })
}
