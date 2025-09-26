//! Definitions for EE message types.

use strata_acct_types::SubjectId;

/// Decoded possible EE account messages we want to honor.
///
/// This is not intended to capture all possible message types.
// TODO make zero copy?
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedEeMessage {
    /// Deposit from L1 to a subject in the EE.
    Deposit(DepositMsgData),

    /// Transfer from a subject in one EE to a subject in another EE.
    SubjTransfer(SubjTransferMsgData),

    /// Commit an update.
    Commit(CommitMsgData),
}

impl DecodedEeMessage {
    /// Decode a raw buffer.
    pub fn decode_raw(buf: &[u8]) -> Option<DecodedEeMessage> {
        // TODO use msg ty fmt
        unimplemented!()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepositMsgData {
    dest_subject: SubjectId,
}

impl DepositMsgData {
    pub fn new(dest_subject: SubjectId) -> Self {
        Self { dest_subject }
    }

    pub fn dest_subject(&self) -> SubjectId {
        self.dest_subject
    }
}

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
#[derive(Clone, Debug, Eq, PartialEq)]
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
