//! Definitions for EE message types.

use strata_acct_types::SubjectId;

/// Describes a transfer between subjects in EEs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubjTransferMsgData {
    source_subject: SubjectId,
    dest_subject: SubjectId,
    transfer_data: Vec<u8>,
}

impl SubjTransferMsgData {
    pub fn new(source_subject: SubjectId, dest_subject: SubjectId, transfer_data: Vec<u8>) -> Self {
        Self {
            source_subject,
            dest_subject,
            transfer_data,
        }
    }

    pub fn source_subject(&self) -> SubjectId {
        self.source_subject
    }

    pub fn dest_subject(&self) -> SubjectId {
        self.dest_subject
    }

    pub fn transfer_data(&self) -> &[u8] {
        &self.transfer_data
    }
}

/// Describes a chunk a sequencer wants to stage.
pub struct CommitMsgData {
    chunk_commitment: [u8; 32],
}

impl CommitMsgData {
    pub fn new(chunk_commitment: [u8; 32]) -> Self {
        Self { chunk_commitment }
    }

    pub fn chunk_commitment(&self) -> [u8; 32] {
        self.chunk_commitment
    }
}
