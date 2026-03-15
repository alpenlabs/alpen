use arbitrary::{Arbitrary, Unstructured};
use bitcoin::OutPoint;
use strata_btc_types::arbitrary_bitcoin;

use crate::slash::SlashTxHeaderAux;

/// Information extracted from a Bitcoin slash transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: SlashTxHeaderAux,
    /// Previous outpoint referenced by the second input (stake connector).
    stake_inpoint: OutPoint,
}

impl<'a> Arbitrary<'a> for SlashInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            header_aux: u.arbitrary()?,
            stake_inpoint: arbitrary_bitcoin::arbitrary_outpoint(u)?,
        })
    }
}

impl SlashInfo {
    pub fn new(header_aux: SlashTxHeaderAux, stake_inpoint: OutPoint) -> Self {
        Self {
            header_aux,
            stake_inpoint,
        }
    }

    pub fn header_aux(&self) -> &SlashTxHeaderAux {
        &self.header_aux
    }

    pub fn stake_inpoint(&self) -> &OutPoint {
        &self.stake_inpoint
    }
}
