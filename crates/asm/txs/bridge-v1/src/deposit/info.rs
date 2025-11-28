use arbitrary::Arbitrary;
use strata_primitives::l1::{BitcoinAmount, BitcoinOutPoint};

use crate::deposit::aux::DepositTxHeaderAux;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct DepositInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: DepositTxHeaderAux,

    /// The amount of Bitcoin deposited.
    amt: BitcoinAmount,

    /// The outpoint of the deposit transaction.
    outpoint: BitcoinOutPoint,
}

impl DepositInfo {
    pub fn new(
        header_aux: DepositTxHeaderAux,
        amt: BitcoinAmount,
        outpoint: BitcoinOutPoint,
    ) -> Self {
        Self {
            header_aux,
            amt,
            outpoint,
        }
    }

    pub fn header_aux(&self) -> &DepositTxHeaderAux {
        &self.header_aux
    }

    pub fn header_aux_mut(&mut self) -> &mut DepositTxHeaderAux {
        &mut self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }

    pub fn set_amt(&mut self, amt: BitcoinAmount) {
        self.amt = amt;
    }

    pub fn outpoint(&self) -> BitcoinOutPoint {
        self.outpoint
    }

    pub fn set_outpoint(&mut self, outpoint: BitcoinOutPoint) {
        self.outpoint = outpoint;
    }
}
