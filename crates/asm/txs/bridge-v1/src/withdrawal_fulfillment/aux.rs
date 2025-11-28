use arbitrary::Arbitrary;
use strata_codec::Codec;

/// Auxiliary data in the SPS-50 header for bridge v1 withdrawal fulfillment transactions.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct WithdrawalFulfillmentTxHeaderAux {
    /// The index of the locked deposit UTXO that the operator will receive payout from.
    /// This index is used to verify that the operator correctly fulfilled their assignment
    /// (correct amount to the correct user within the assigned deadline). Upon successful
    /// verification against the state's assignments table, the operator is authorized to
    /// claim the payout from this deposit.
    deposit_idx: u32,
}

impl WithdrawalFulfillmentTxHeaderAux {
    pub fn new(deposit_idx: u32) -> Self {
        Self { deposit_idx }
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }

    #[cfg(feature = "test-utils")]
    pub fn set_deposit_idx(&mut self, deposit_idx: u32) {
        self.deposit_idx = deposit_idx;
    }
}
