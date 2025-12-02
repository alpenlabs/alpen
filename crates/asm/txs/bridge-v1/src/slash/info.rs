use arbitrary::Arbitrary;
use strata_primitives::l1::BitcoinOutPoint;

use crate::slash::SlashTxHeaderAux;

/// Information extracted from a Bitcoin slash transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct SlashInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: SlashTxHeaderAux,
    /// Previous outpoint referenced second input (stake connector).
    second_inpoint: BitcoinOutPoint,
}

impl SlashInfo {
    pub fn new(header_aux: SlashTxHeaderAux, second_inpoint: BitcoinOutPoint) -> Self {
        Self {
            header_aux,
            second_inpoint,
        }
    }

    pub fn header_aux(&self) -> &SlashTxHeaderAux {
        &self.header_aux
    }

    pub fn second_input_outpoint(&self) -> &BitcoinOutPoint {
        &self.second_inpoint
    }
}
