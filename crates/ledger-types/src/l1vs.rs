//! L1 view state.
// TODO should this be renamed to "epochal state" since we encompasses a little
// more than just L1 state like this current epoch (which we include here since
// it's only updated at the epoch boundaries anyways)

use strata_acct_types::BitcoinAmount;
pub use strata_asm_common::AsmManifest;
pub use strata_identifiers::{EpochCommitment, L1BlockId, L1Height};

/// State relating to the L1 view.
///
/// This is only updated in the sealing phase at epoch boundaries, so it has no
/// DA footprint.  There will probably also be some extra things in here that
/// have a similar data path without DA footprint.
pub trait IL1ViewState: Clone {
    /// Gets the current epoch.
    fn cur_epoch(&self) -> u32;

    /// Sets the current epoch.
    fn set_cur_epoch(&mut self, epoch: u32);

    /// Last L1 block ID.
    fn last_l1_blkid(&self) -> &L1BlockId;

    /// Last L1 block height.
    fn last_l1_height(&self) -> L1Height;

    /// Appends a new ASM manifest to the accumulator, also updating the last L1
    /// block height and other fields.
    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest);

    /// Gets the field for the epoch that the ASM considers to be valid.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    fn asm_recorded_epoch(&self) -> &EpochCommitment;

    /// Sets the field for the epoch that the ASM considers to be finalized.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment);

    /// Gets the total OL ledger balance.
    fn total_ledger_balance(&self) -> BitcoinAmount;

    /// Sets the total OL ledger balance.
    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount);
}
