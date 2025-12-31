//! Interpretation of extra data.

use strata_acct_types::Hash;
use strata_codec::impl_type_flat_struct;

impl_type_flat_struct! {
    /// Extra data for state updates. Describes the new execution tip and queue removals.
    ///
    /// Used at block, chunk, and batch levels.
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
    pub struct UpdateExtraData {
        /// The blkid of the new execution tip block.
        new_tip_blkid: Hash,

        /// The total number of items to remove from the input queue.
        processed_inputs: u32,

        /// The total number of items to remove from the fincl queue.
        processed_fincls: u32,
    }
}
