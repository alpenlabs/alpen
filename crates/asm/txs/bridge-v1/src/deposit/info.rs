use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{OutPoint, ScriptBuf, TxOut};
use strata_btc_types::arbitrary_bitcoin;
use strata_primitives::l1::BitcoinAmount;

use crate::deposit::aux::DepositTxHeaderAux;

/// Information extracted from a deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: DepositTxHeaderAux,

    /// The deposit output containing the deposited amount and its locking script.
    deposit_output: TxOut,

    /// Previous outpoint referenced by the DT input.
    drt_inpoint: OutPoint,
}

impl<'a> Arbitrary<'a> for DepositInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            header_aux: u.arbitrary()?,
            deposit_output: arbitrary_bitcoin::arbitrary_txout(u)?,
            drt_inpoint: arbitrary_bitcoin::arbitrary_outpoint(u)?,
        })
    }
}

impl DepositInfo {
    pub fn new(
        header_aux: DepositTxHeaderAux,
        deposit_output: TxOut,
        drt_inpoint: OutPoint,
    ) -> Self {
        Self {
            header_aux,
            deposit_output,
            drt_inpoint,
        }
    }

    pub fn header_aux(&self) -> &DepositTxHeaderAux {
        &self.header_aux
    }

    pub fn drt_inpoint(&self) -> &OutPoint {
        &self.drt_inpoint
    }

    #[cfg(feature = "test-utils")]
    pub fn header_aux_mut(&mut self) -> &mut DepositTxHeaderAux {
        &mut self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.deposit_output.value.into()
    }

    #[cfg(feature = "test-utils")]
    pub fn set_amt(&mut self, amt: BitcoinAmount) {
        self.deposit_output.value = amt.into();
    }

    pub fn locked_script(&self) -> &ScriptBuf {
        &self.deposit_output.script_pubkey
    }

    #[cfg(feature = "test-utils")]
    pub fn set_locked_script(&mut self, new_script_pubkey: ScriptBuf) {
        self.deposit_output.script_pubkey = new_script_pubkey;
    }
}
