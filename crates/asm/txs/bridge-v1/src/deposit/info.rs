use arbitrary::Arbitrary;
use bitcoin::{OutPoint, ScriptBuf, TxOut};
use strata_primitives::l1::{BitcoinAmount, BitcoinOutPoint, BitcoinTxOut};

use crate::deposit::aux::DepositTxHeaderAux;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct DepositInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: DepositTxHeaderAux,

    /// The amount of Bitcoin deposited.
    deposit_out: BitcoinTxOut,

    /// The outpoint of the deposit transaction.
    outpoint: BitcoinOutPoint,

    drt_inpoint: BitcoinOutPoint,
}

impl DepositInfo {
    pub fn new(
        header_aux: DepositTxHeaderAux,
        deposit_out: BitcoinTxOut,
        outpoint: BitcoinOutPoint,
        drt_inpoint: BitcoinOutPoint,
    ) -> Self {
        Self {
            header_aux,
            deposit_out,
            outpoint,
            drt_inpoint,
        }
    }

    pub fn drt_inpoint(&self) -> &OutPoint {
        &self.drt_inpoint.0
    }

    pub fn header_aux(&self) -> &DepositTxHeaderAux {
        &self.header_aux
    }

    #[cfg(feature = "test-utils")]
    pub fn header_aux_mut(&mut self) -> &mut DepositTxHeaderAux {
        &mut self.header_aux
    }

    pub fn amt(&self) -> BitcoinAmount {
        TxOut::from(self.deposit_out.clone()).value.into()
    }

    pub fn locked_script(&self) -> &ScriptBuf {
        &self.deposit_out.inner().script_pubkey
    }

    #[cfg(feature = "test-utils")]
    pub fn set_amt(&mut self, amt: BitcoinAmount) {
        let new_tx_out = TxOut {
            script_pubkey: self.locked_script().clone(),
            value: amt.into(),
        };
        self.deposit_out = new_tx_out.into()
    }

    pub fn outpoint(&self) -> BitcoinOutPoint {
        self.outpoint
    }

    #[cfg(feature = "test-utils")]
    pub fn set_outpoint(&mut self, outpoint: BitcoinOutPoint) {
        self.outpoint = outpoint;
    }
}
