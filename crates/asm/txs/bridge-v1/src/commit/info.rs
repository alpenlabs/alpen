use arbitrary::{Arbitrary, Unstructured};
use bitcoin::ScriptBuf;
use strata_primitives::l1::BitcoinOutPoint;

use crate::commit::aux::CommitTxHeaderAux;

/// Information extracted from a Bitcoin commit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// Parsed SPS-50 auxiliary data.
    header_aux: CommitTxHeaderAux,

    /// The outpoint spent by the first input.
    /// Must be validated that it spends from an N/N-locked output during transaction validation.
    first_input_outpoint: BitcoinOutPoint,

    /// The script from the second output (index 1).
    /// Must be validated as N/N-locked during transaction validation.
    second_output_script: ScriptBuf,
}

impl CommitInfo {
    pub fn new(
        header_aux: CommitTxHeaderAux,
        first_input_outpoint: BitcoinOutPoint,
        second_output_script: ScriptBuf,
    ) -> Self {
        Self {
            header_aux,
            first_input_outpoint,
            second_output_script,
        }
    }

    pub fn header_aux(&self) -> &CommitTxHeaderAux {
        &self.header_aux
    }

    pub fn header_aux_mut(&mut self) -> &mut CommitTxHeaderAux {
        &mut self.header_aux
    }

    pub fn first_input_outpoint(&self) -> &BitcoinOutPoint {
        &self.first_input_outpoint
    }

    pub fn second_output_script(&self) -> &ScriptBuf {
        &self.second_output_script
    }
}

impl<'a> Arbitrary<'a> for CommitInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let header_aux = CommitTxHeaderAux::arbitrary(u)?;
        let first_input_outpoint = BitcoinOutPoint::arbitrary(u)?;
        let second_output_script = ScriptBuf::new();

        Ok(CommitInfo {
            header_aux,
            first_input_outpoint,
            second_output_script,
        })
    }
}
