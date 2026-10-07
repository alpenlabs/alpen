//! The chain tip the mempool admits transactions for, and the spec of the
//! block after it.

use std::convert::Infallible;

use strata_identifiers::{Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment};
use strata_ol_state_types::IStateAccessor;
use strata_ol_stf::{
    EpochL1Range, EpochSpecSelectionError, OLSpecId, UpgradeRequired, select_next_epoch_spec,
};
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

/// Selects the spec of the block after `tip`, whose state is `tip_state`, as
/// block assembly selects it.
///
/// A block after a terminal tip starts the next epoch, whose spec
/// [`select_next_epoch_spec`] selects from the tip's epoch: its L1 range
/// starts at the last L1 block of the epoch its own summary links to, and the
/// genesis epoch starts at `genesis_l1_block`. Any other block continues the
/// tip's epoch under the spec the tip state was produced under.
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
    let Some(epoch) = tip.ends_epoch() else {
        return Ok(Ok(tip_versions.cur_spec()));
    };
    let tip_last_l1 =
        L1BlockCommitment::new(tip_state.last_l1_height(), *tip_state.last_l1_blkid());

    let prev_last = if epoch == 0 {
        genesis_l1_block
    } else {
        let tip_epoch = EpochCommitment::from_terminal(epoch, tip.block());
        let summary = storage
            .ol_checkpoint()
            .get_epoch_summary_async(tip_epoch)
            .await?
            .ok_or(OLMempoolError::MissingEpochSummary(tip_epoch))?;
        let prev = summary
            .get_prev_epoch_commitment()
            .expect("a summary of an epoch after genesis has a previous epoch");
        *storage
            .ol_checkpoint()
            .get_epoch_summary_async(prev)
            .await?
            .ok_or(OLMempoolError::MissingEpochSummary(prev))?
            .new_l1()
    };
    let range = EpochL1Range::new(prev_last, tip_last_l1)?;
    let last_manifest = match range.last_processed() {
        Some(block) => storage.l1().get_block_manifest_async(block.blkid()).await?,
        None => None,
    };
    match select_next_epoch_spec(tip_versions, range, |_| Ok::<_, Infallible>(last_manifest)) {
        Ok(spec) => Ok(Ok(spec)),
        Err(EpochSpecSelectionError::UpgradeRequired(upgrade)) => Ok(Err(upgrade)),
        Err(err) => Err(OLMempoolError::SpecSelection(err)),
    }
}
