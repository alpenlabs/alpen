//! Snark account types.

use strata_acct_types::AcctId;

// TODO use actual MMR proofs
type MmrProof = Vec<u8>;

/// Message entry in an account inbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntry {
    source: AcctId,
    incl_epoch: u32,
    payload: Vec<u8>,
}

impl MessageEntry {
    pub fn new(source: AcctId, incl_epoch: u32, payload: Vec<u8>) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }

    pub fn source(&self) -> AcctId {
        self.source
    }

    pub fn incl_epoch(&self) -> u32 {
        self.incl_epoch
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntryProof {
    entry: MessageEntry,
    proof: MmrProof,
}

impl MessageEntryProof {
    pub fn new(entry: MessageEntry, proof: MmrProof) -> Self {
        Self { entry, proof }
    }

    pub fn entry(&self) -> &MessageEntry {
        &self.entry
    }

    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}
