use arbitrary::Arbitrary;
use strata_primitives::l1::BitcoinAmount;

use crate::deposit_request::DrtHeaderAux;

/// Information extracted from a deposit request transaction.
#[derive(Debug, Clone, Arbitrary)]
pub struct DrtInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: DrtHeaderAux,
    amt: BitcoinAmount,
}

impl DrtInfo {
    pub fn new(header_aux: DrtHeaderAux, amt: BitcoinAmount) -> Self {
        Self { header_aux, amt }
    }

    pub fn header_aux(&self) -> &DrtHeaderAux {
        &self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}
