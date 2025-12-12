use arbitrary::Arbitrary;
use strata_bridge_types::OperatorIdx;
use strata_codec::Codec;

/// Auxiliary data in the SPS-50 header for bridge v1 unstake transaction.
///
/// This represents the type-specific auxiliary bytes that appear after the magic, subprotocol,
/// and tx_type fields in the OP_RETURN output at position 0.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary, Codec)]
pub struct UnstakeTxHeaderAux {
    /// The index of the operator whose stake is being unlocked.
    operator_idx: OperatorIdx,
}

impl UnstakeTxHeaderAux {
    pub fn new(operator_idx: OperatorIdx) -> Self {
        Self { operator_idx }
    }

    pub fn operator_idx(&self) -> OperatorIdx {
        self.operator_idx
    }
}
