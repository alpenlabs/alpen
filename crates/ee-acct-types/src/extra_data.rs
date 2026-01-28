//! Interpretation of extra data.

use strata_acct_types::Hash;
use strata_codec::impl_type_flat_struct;
use strata_snark_acct_runtime::IExtraData;

impl_type_flat_struct! {
    /// Message sent in the extra data field in the update operation.
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

impl IExtraData for UpdateExtraData {}
