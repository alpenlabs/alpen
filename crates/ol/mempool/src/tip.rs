//! The chain tip the mempool admits transactions for, and the spec of the
//! block after it.

use strata_identifiers::{Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment};
use strata_ol_spec_selection::{
    SelectSpecAfterEpochError, select_block_spec_async, select_spec_after_unsummarized_epoch_async,
};
use strata_ol_state_types::IStateAccessor;
use strata_ol_stf::{EpochSpecSelectionError, OLSpecId, UpgradeRequired};
use strata_status::OLSyncStatus;
use strata_storage::NodeStorage;

use crate::{OLMempoolError, OLMempoolResult};

/// The chain tip the mempool admits transactions for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MempoolTip {
    block: OLBlockCommitment,
    ends_epoch: Option<Epoch>,
}

impl MempoolTip {
    /// Creates a tip at `block`, which ends epoch `ends_epoch` if it is an
    /// epoch's terminal block.
    pub fn new(block: OLBlockCommitment, ends_epoch: Option<Epoch>) -> Self {
        Self { block, ends_epoch }
    }

    /// Creates the tip of the OL sync status the fork-choice manager
    /// publishes, which carries whether the tip ends its epoch.
    pub fn from_sync_status(status: &OLSyncStatus) -> Self {
        Self::new(
            status.tip(),
            status.tip_is_terminal().then_some(status.tip_epoch()),
        )
    }

    /// Returns the tip block.
    pub fn block(&self) -> OLBlockCommitment {
        self.block
    }

    /// Returns the epoch the tip ends, if it is an epoch's terminal block.
    pub fn ends_epoch(&self) -> Option<Epoch> {
        self.ends_epoch
    }
}

/// Selects the spec of the block after `tip`, whose state is `tip_state`, with
/// [`select_block_spec_async`], as block assembly selects it.
///
/// The mempool launches before FCM repairs a summary that a crash left
/// missing, so a terminal tip's own summary can be absent. Its epoch's L1 range
/// then starts at the canonical summary of the epoch before, as
/// [`select_spec_after_unsummarized_epoch_async`] describes.
///
/// Returns the [`UpgradeRequired`] outcome as a value, since the mempool keeps
/// running and only stops admitting transactions.
///
/// # Errors
///
/// Returns an error if the tip epoch's summaries or last manifest are missing
/// or inconsistent.
pub(crate) async fn select_tip_spec(
    storage: &NodeStorage,
    genesis_l1_block: L1BlockCommitment,
    tip: MempoolTip,
    tip_state: &impl IStateAccessor,
) -> OLMempoolResult<Result<OLSpecId, UpgradeRequired>> {
    let tip_versions = tip_state.spec_versions();
    let tip_last_l1 =
        L1BlockCommitment::new(tip_state.last_l1_height(), *tip_state.last_l1_blkid());
    let tip_epoch = tip
        .ends_epoch()
        .map(|epoch| EpochCommitment::from_terminal(epoch, tip.block()));

    let checkpoints = storage.ol_checkpoint();
    let get_manifest = |block: L1BlockCommitment| async move {
        storage.l1().get_block_manifest_async(block.blkid()).await
    };
    let selected = match select_block_spec_async(
        tip_epoch,
        tip_versions,
        tip_last_l1,
        genesis_l1_block,
        |epoch| async move { checkpoints.get_epoch_summary_async(epoch).await },
        get_manifest,
    )
    .await
    {
        Err(SelectSpecAfterEpochError::MissingEpochSummary(missing))
            if Some(missing) == tip_epoch =>
        {
            let prev_epoch_summary = match checkpoints
                .get_canonical_epoch_commitment_at_async(missing.epoch() - 1)
                .await?
            {
                Some(prev_epoch) => checkpoints.get_epoch_summary_async(prev_epoch).await?,
                None => None,
            };
            match prev_epoch_summary {
                Some(prev_epoch_summary) => {
                    select_spec_after_unsummarized_epoch_async(
                        tip_versions,
                        tip_last_l1,
                        &prev_epoch_summary,
                        get_manifest,
                    )
                    .await
                }
                None => Err(SelectSpecAfterEpochError::MissingEpochSummary(missing)),
            }
        }
        selected => selected,
    };
    match selected {
        Ok(spec) => Ok(Ok(spec)),
        Err(err) => match err.into_lookup_error() {
            Ok(lookup) => Err(lookup.into()),
            Err(SelectSpecAfterEpochError::Selection(
                EpochSpecSelectionError::UpgradeRequired(upgrade),
            )) => Ok(Err(upgrade)),
            Err(err) => Err(OLMempoolError::SpecSelection(err)),
        },
    }
}
