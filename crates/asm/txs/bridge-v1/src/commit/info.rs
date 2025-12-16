use arbitrary::{Arbitrary, Unstructured};
use bitcoin::ScriptBuf;
use strata_primitives::l1::BitcoinOutPoint;

use crate::commit::aux::CommitTxHeaderAux;

/// Information extracted from a Bitcoin commit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: CommitTxHeaderAux,

    /// The outpoint spent by the stake connector input.
    /// Must be validated that it spends from an N/N-locked output during transaction validation.
    stake_inpoint: BitcoinOutPoint,

    /// The script from the second output (index 1).
    /// Must be validated as N/N-locked during transaction validation.
    nn_script: ScriptBuf,
}

impl CommitInfo {
    pub fn new(
        header_aux: CommitTxHeaderAux,
        stake_inpoint: BitcoinOutPoint,
        nn_script: ScriptBuf,
    ) -> Self {
        Self {
            header_aux,
            stake_inpoint,
            nn_script,
        }
    }

    pub fn header_aux(&self) -> &CommitTxHeaderAux {
        &self.header_aux
    }

    #[cfg(feature = "test-utils")]
    pub fn header_aux_mut(&mut self) -> &mut CommitTxHeaderAux {
        &mut self.header_aux
    }

    pub fn stake_inpoint(&self) -> &BitcoinOutPoint {
        &self.stake_inpoint
    }

    pub fn nn_script(&self) -> &ScriptBuf {
        &self.nn_script
    }
}

impl<'a> Arbitrary<'a> for CommitInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let header_aux = CommitTxHeaderAux::arbitrary(u)?;
        let stake_inpoint = BitcoinOutPoint::arbitrary(u)?;
        let nn_script = ScriptBuf::new();

        Ok(CommitInfo {
            header_aux,
            stake_inpoint,
            nn_script,
        })
    }
}
