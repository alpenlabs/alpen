use arbitrary::Arbitrary;
use strata_primitives::l1::{BitcoinAmount, BitcoinTxOut};

use crate::deposit::aux::DepositTxHeaderAux;

/// Information extracted from a deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct DepositInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: DepositTxHeaderAux,

    /// The deposit output containing the deposited amount and its locking script.
    deposit_output: BitcoinTxOut,
}

impl DepositInfo {
    pub fn new(header_aux: DepositTxHeaderAux, txout: BitcoinTxOut) -> Self {
        Self {
            header_aux,
            deposit_output: txout,
        }
    }

    pub fn header_aux(&self) -> &DepositTxHeaderAux {
        &self.header_aux
    }

    #[cfg(feature = "test-utils")]
    pub fn header_aux_mut(&mut self) -> &mut DepositTxHeaderAux {
        &mut self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.deposit_output.inner().value.into()
    }

    #[cfg(feature = "test-utils")]
    pub fn set_amt(&mut self, amt: BitcoinAmount) {
        use bitcoin::TxOut;

        let txout = self.deposit_output.inner().clone();
        let new_txout = TxOut {
            value: amt.into(),
            script_pubkey: txout.script_pubkey,
        };
        self.deposit_output = new_txout.into();
    }
}
