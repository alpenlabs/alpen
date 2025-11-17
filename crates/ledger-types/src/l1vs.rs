//! L1 view state.
// TODO should this be renamed to "epochal state" since we encompasses a little
// more than just L1 state like this current epoch (which we include here since
// it's only updated at the epoch boundaries anyways)

use strata_acct_types::{BitcoinAmount, Mmr64};
pub use strata_asm_common::AsmManifest;
pub use strata_identifiers::{EpochCommitment, L1BlockId, L1Height};
use strata_primitives::Epoch;

/// State relating to the L1 view.
///
/// This is only updated in the sealing phase at epoch boundaries, so it has no
/// DA footprint.  There will probably also be some extra things in here that
/// have a similar data path without DA footprint.
pub trait IL1ViewState {
    /// Gets the current epoch.
    fn cur_epoch(&self) -> Epoch;

    /// Sets the current epoch.
    fn set_cur_epoch(&mut self, epoch: Epoch);

    /// Last L1 block ID.
    fn last_l1_blkid(&self) -> &L1BlockId;

    /// Sets Last L1 block ID.
    fn set_last_l1_blkid(&mut self, blkid: L1BlockId);

    /// Last L1 block height.
    fn last_l1_height(&self) -> L1Height;

    /// Sets Last L1 block height
    fn set_last_l1_height(&mut self, height: L1Height);

    /// Appends a new ASM manifest to the accumulator, also updating the last L1
    /// block height and other fields.
    fn append_manifest(&mut self, mf: AsmManifest);

    /// Gets the MMR for asm manifests.
    ///
    /// This is for the accounts to make references about L1 blocks.
    fn asm_manifests_mmr(&self) -> &Mmr64;

    /// Gets the field for the epoch that the ASM considers to be valid.
    ///
    /// This is our perspective of the last block's ASM manifest we've accepted.
    fn asm_recorded_epoch(&self) -> &EpochCommitment;

    /// Sets the field for the epoch that the ASM considers to be finalized.
    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment);

    /// Gets the total OL ledger balance.
    fn total_ledger_balance(&self) -> BitcoinAmount;

    /// Increments the total OL ledger balance.
    fn increment_total_ledger_balance(&mut self, amt: BitcoinAmount) -> BitcoinAmount;

    /// Decrements the total OL ledger balance.
    fn decrement_total_ledger_balance(&mut self, amt: BitcoinAmount) -> BitcoinAmount;
}
