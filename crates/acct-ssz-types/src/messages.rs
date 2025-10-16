// Message types from the SSZ schema.
// Types defined here match the pythonic schema in `schemas/acct-types.ssz`.

use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use tree_hash_derive::TreeHash as TreeHashDerive;

use crate::{AccountId, BitcoinAmount, MAX_MSG_PAYLOAD_DATA_BYTES};

/// Variable-length byte list for message payload data
pub type MsgPayloadData = VariableList<u8, MAX_MSG_PAYLOAD_DATA_BYTES>;

/// Message payload (value + arbitrary data)
/// Schema: class MsgPayload(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct MsgPayload {
    pub value: BitcoinAmount,
    pub data: MsgPayloadData,
}

/// Outgoing message from one account to another
/// Schema: class SentMessage(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct SentMessage {
    pub dest: AccountId,
    pub payload: MsgPayload,
}

/// Incoming message received by an account
/// Schema: class ReceivedMessage(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct ReceivedMessage {
    pub source: AccountId,
    pub payload: MsgPayload,
}
