use arbitrary::Arbitrary;
use strata_primitives::l1::{BitcoinAmount, BitcoinTxOut};

use crate::deposit_request::DrtHeaderAux;

/// Information extracted from a deposit request transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct DepositRequestInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: DrtHeaderAux,

    /// The deposit request output containing the amount and its locking script.
    deposit_request_output: BitcoinTxOut,
}

impl DepositRequestInfo {
    pub fn new(header_aux: DrtHeaderAux, deposit_request_output: BitcoinTxOut) -> Self {
        Self {
            header_aux,
            deposit_request_output,
        }
    }

    pub fn header_aux(&self) -> &DrtHeaderAux {
        &self.header_aux
    }

    pub fn deposit_request_output(&self) -> &BitcoinTxOut {
        &self.deposit_request_output
    }

    #[cfg(feature = "test-utils")]
    pub fn header_aux_mut(&mut self) -> &mut DrtHeaderAux {
        &mut self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.deposit_request_output.inner().value.into()
    }

    #[cfg(feature = "test-utils")]
    pub fn set_amt(&mut self, amt: BitcoinAmount) {
        use bitcoin::TxOut;

        let txout = self.deposit_request_output.inner().clone();
        let new_txout = TxOut {
            value: amt.into(),
            script_pubkey: txout.script_pubkey,
        };
        self.deposit_request_output = new_txout.into();
    }
}
