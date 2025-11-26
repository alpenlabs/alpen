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
    pub deposit_idx: u32,
}
