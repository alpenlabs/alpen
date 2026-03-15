use arbitrary::{Arbitrary, Unstructured};
use bitcoin::TxOut;
use strata_btc_types::arbitrary_bitcoin;
use strata_primitives::l1::BitcoinAmount;

use crate::deposit_request::DrtHeaderAux;

/// Information extracted from a deposit request transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositRequestInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: DrtHeaderAux,

    /// The deposit request output containing the amount and its locking script.
    deposit_request_output: TxOut,
}

impl<'a> Arbitrary<'a> for DepositRequestInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            header_aux: u.arbitrary()?,
            deposit_request_output: arbitrary_bitcoin::arbitrary_txout(u)?,
        })
    }
}

impl DepositRequestInfo {
    pub fn new(header_aux: DrtHeaderAux, deposit_request_output: TxOut) -> Self {
        Self {
            header_aux,
            deposit_request_output,
        }
    }

    pub fn header_aux(&self) -> &DrtHeaderAux {
        &self.header_aux
    }

    pub fn deposit_request_output(&self) -> &TxOut {
        &self.deposit_request_output
    }

    #[cfg(feature = "test-utils")]
    pub fn header_aux_mut(&mut self) -> &mut DrtHeaderAux {
        &mut self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.deposit_request_output.value.into()
    }

    #[cfg(feature = "test-utils")]
    pub fn set_amt(&mut self, amt: BitcoinAmount) {
        self.deposit_request_output.value = amt.into();
    }
}
