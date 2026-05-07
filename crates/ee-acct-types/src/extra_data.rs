//! Interpretation of extra data.

use strata_acct_types::Hash;
use strata_codec::impl_type_flat_struct;
use strata_snark_acct_runtime::IExtraData;

impl_type_flat_struct! {
    /// Message sent in the extra data field in the update operation.
    ///
    /// TODO(STR-TBD): `new_tip_state_root` is required by EE proof/reconstruction so
    /// [`EeAccountState`] can commit to the full last execution block state.
    /// However, OL currently copies SAU `extra_data` byte-for-byte into
    /// `SnarkAccountUpdateLogData`, which means this field is also published in
    /// OL DA logs for every SAU. OL needs the same opaque `extra_data` bytes in
    /// `UpdateProofPubParams` to verify the EE account proof, but the generic
    /// OL STF does not know the EE-specific extra-data type and cannot safely
    /// strip this field before emitting logs. Fixing that requires a proper
    /// design across OL tracker reconstruction, the EE account proof statement,
    /// and the EE account runtime/OL boundary so log projection does not weaken
    /// proof binding or hardcode EE semantics inside generic OL logic.
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
    pub struct UpdateExtraData {
        /// The blkid of the new execution tip block.
        new_tip_blkid: Hash,

        /// The EVM state root of the new execution tip block.
        new_tip_state_root: Hash,

        /// The total number of items to remove from the input queue.
        processed_inputs: u32,

        /// The total number of items to remove from the fincl queue.
        processed_fincls: u32,
    }
}

impl IExtraData for UpdateExtraData {}
