use arbitrary::Arbitrary;

use crate::unstake::UnstakeTxHeaderAux;

/// Information extracted from an unstake transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct UnstakeInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: UnstakeTxHeaderAux,
}

impl UnstakeInfo {
    pub fn new(header_aux: UnstakeTxHeaderAux) -> Self {
        Self { header_aux }
    }

    pub fn header_aux(&self) -> &UnstakeTxHeaderAux {
        &self.header_aux
    }
}
