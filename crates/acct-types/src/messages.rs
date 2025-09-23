//! Account message types.

use crate::id::AcctId;

/// General message type, not designed for a particular context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcctMessage {
    sender: AcctId,
    receiver: AcctId,
    data: MessageData,
}

/// "Contents" of a message, the payload and sent value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageData {
    value: u64, // TODO convert to BitcoinAmount
    payload: Vec<u8>,
}
