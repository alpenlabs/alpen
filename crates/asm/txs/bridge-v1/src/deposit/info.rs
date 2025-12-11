use arbitrary::Arbitrary;
use strata_primitives::l1::BitcoinAmount;

use crate::deposit::aux::DepositTxHeaderAux;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct DepositInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: DepositTxHeaderAux,

    /// The amount of Bitcoin deposited.
    amt: BitcoinAmount,
}

impl DepositInfo {
    pub fn new(header_aux: DepositTxHeaderAux, amt: BitcoinAmount) -> Self {
        Self { header_aux, amt }
    }

    pub fn header_aux(&self) -> &DepositTxHeaderAux {
        &self.header_aux
    }

    #[cfg(feature = "test-utils")]
    pub fn header_aux_mut(&mut self) -> &mut DepositTxHeaderAux {
        &mut self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }

    #[cfg(feature = "test-utils")]
    pub fn set_amt(&mut self, amt: BitcoinAmount) {
        self.amt = amt;
    }
}
