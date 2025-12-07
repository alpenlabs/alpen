use arbitrary::Arbitrary;
use strata_primitives::l1::BitcoinOutPoint;

use crate::unstake::UnstakeTxHeaderAux;

/// Information extracted from an unstake transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct UnstakeInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: UnstakeTxHeaderAux,
    /// Previous outpoint referenced second input (stake connector).
    second_inpoint: BitcoinOutPoint,
}

impl UnstakeInfo {
    pub fn new(header_aux: UnstakeTxHeaderAux, second_inpoint: BitcoinOutPoint) -> Self {
        Self {
            header_aux,
            second_inpoint,
        }
    }

    pub fn header_aux(&self) -> &UnstakeTxHeaderAux {
        &self.header_aux
    }

    pub fn second_inpoint(&self) -> &BitcoinOutPoint {
        &self.second_inpoint
    }
}
