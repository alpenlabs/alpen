//! Snark account types.

use strata_acct_types::AcctId;

/// Message entry in an account inbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntry {
    source: AcctId,
    incl_epoch: u32,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntryProof {
    // TODO
}
