use arbitrary::Arbitrary;
use bitcoin::{ScriptBuf, TxOut};
use strata_primitives::l1::{BitcoinAmount, BitcoinScriptBuf, BitcoinTxOut};

use crate::deposit_request::DrtHeaderAux;

/// Information extracted from a deposit request transaction.
#[derive(Debug, Clone, Arbitrary)]
pub struct DrtInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: DrtHeaderAux,

    drt_out: BitcoinTxOut,
}

impl DrtInfo {
    pub fn new(header_aux: DrtHeaderAux, drt_out: BitcoinTxOut) -> Self {
        Self {
            header_aux,
            drt_out,
        }
    }

    pub fn header_aux(&self) -> &DrtHeaderAux {
        &self.header_aux
    }
}
