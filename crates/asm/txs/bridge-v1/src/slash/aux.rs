use arbitrary::Arbitrary;
use strata_bridge_types::OperatorIdx;
use strata_codec::Codec;

/// Auxiliary data in the SPS-50 header for bridge v1 slash transaction.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct SlashTxHeaderAux {
    /// The index of the operator being slashed.
    pub operator_idx: OperatorIdx,
}
