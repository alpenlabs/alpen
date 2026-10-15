use std::future::Future;

use strata_db_types::DbResult;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, Slot};

/// Provides canonical block and completed-epoch queries.
///
/// A canonical commitment does not guarantee locally available block data or state.
pub trait OLCanonicalChainProvider: Send + Sync + 'static {
    /// Returns the canonical block commitment at `slot`.
    ///
    /// Returns `None` when there is no canonical block at that slot, including sparse gaps.
    fn get_canonical_block_at_async(
        &self,
        slot: Slot,
    ) -> impl Future<Output = DbResult<Option<OLBlockCommitment>>> + Send;

    /// Blocking counterpart of [`Self::get_canonical_block_at_async`].
    fn get_canonical_block_at_blocking(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>>;

    /// Returns the canonical tip.
    ///
    /// Returns `None` before the canonical chain is initialized.
    fn get_canonical_tip_async(
        &self,
    ) -> impl Future<Output = DbResult<Option<OLBlockCommitment>>> + Send;

    /// Blocking counterpart of [`Self::get_canonical_tip_async`].
    fn get_canonical_tip_blocking(&self) -> DbResult<Option<OLBlockCommitment>>;

    /// Returns the canonical commitment for `epoch`.
    ///
    /// Returns `None` when there is no canonical commitment for `epoch`, including while
    /// `epoch` is still in progress.
    fn get_canonical_epoch_commitment_at_async(
        &self,
        epoch: Epoch,
    ) -> impl Future<Output = DbResult<Option<EpochCommitment>>> + Send;

    /// Blocking counterpart of [`Self::get_canonical_epoch_commitment_at_async`].
    fn get_canonical_epoch_commitment_at_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>>;
}
