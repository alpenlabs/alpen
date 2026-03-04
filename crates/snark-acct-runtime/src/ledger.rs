//! Ledger-related types for snark account update proofs.

use strata_snark_acct_types::{LedgerRefs, UpdateOutputs, UpdateProofPubParams};

/// Info about how the update interacts with the ledger, being checked by the
/// proof.
#[derive(Copy, Clone, Debug)]
pub struct UpdateLedgerInfo<'u> {
    ledger_refs: &'u LedgerRefs,
    outputs: &'u UpdateOutputs,
}

impl<'u> UpdateLedgerInfo<'u> {
    pub fn new(ledger_refs: &'u LedgerRefs, outputs: &'u UpdateOutputs) -> Self {
        Self {
            ledger_refs,
            outputs,
        }
    }

    /// Creates a new instance by extracting the refs from the proof pub params.
    pub fn from_update(update: &'u UpdateProofPubParams) -> Self {
        Self::new(update.ledger_refs(), update.outputs())
    }

    /// Gets a ref to the ledger refs attested to in the update.
    pub fn ledger_refs(&self) -> &'u LedgerRefs {
        self.ledger_refs
    }

    /// Gets a ref to the outputs produced by the update.
    pub fn outputs(&self) -> &'u UpdateOutputs {
        self.outputs
    }
}
