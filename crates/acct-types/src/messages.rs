//! Account message types.

use crate::id::AcctId;

/// Describes a message we're getting ready to send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentMessage {
    dest: AcctId,
    payload: MsgPayload,
}

impl SentMessage {
    pub fn new(dest: AcctId, payload: MsgPayload) -> Self {
        Self { dest, payload }
    }

    pub fn dest(&self) -> AcctId {
        self.dest
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

/// Describes a message being received by an account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedMessage {
    source: AcctId,
    payload: MsgPayload,
}

impl ReceivedMessage {
    pub fn new(source: AcctId, payload: MsgPayload) -> Self {
        Self { source, payload }
    }

    pub fn source(&self) -> AcctId {
        self.source
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

/// Contents of a message, ie the data and sent value payload components.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MsgPayload {
    value: u64, // TODO convert to BitcoinAmount
    data: Vec<u8>,
}

impl MsgPayload {
    pub fn new(value: u64, data: Vec<u8>) -> Self {
        Self { value, data }
    }

    pub fn value(&self) -> u64 {
        self.value
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}
