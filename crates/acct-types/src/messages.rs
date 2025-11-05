//! Account message types.

use crate::{AccountId, BitcoinAmount};

/// Describes a message we're getting ready to send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentMessage {
    /// Destination orchestration layer account ID.
    dest: AccountId,

    /// Message payload.
    payload: MsgPayload,
}

impl SentMessage {
    pub fn new(dest: AccountId, payload: MsgPayload) -> Self {
        Self { dest, payload }
    }

    pub fn dest(&self) -> AccountId {
        self.dest
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

/// Describes a message being received by an account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedMessage {
    source: AccountId,
    payload: MsgPayload,
}

impl ReceivedMessage {
    pub fn new(source: AccountId, payload: MsgPayload) -> Self {
        Self { source, payload }
    }

    pub fn source(&self) -> AccountId {
        self.source
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }
}

/// Contents of a message, ie the data and sent value payload components.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MsgPayload {
    value: BitcoinAmount,
    data: Vec<u8>,
}

impl MsgPayload {
    pub fn new(value: BitcoinAmount, data: Vec<u8>) -> Self {
        Self { value, data }
    }

    pub fn value(&self) -> BitcoinAmount {
        self.value
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}
