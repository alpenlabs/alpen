use strata_acct_types::{AccountId, BitcoinAmount};
use strata_snark_acct_types::MessageEntry;

use crate::traits::IAcctMsg;

/// Meta fields extracted from a message.
#[derive(Copy, Clone, Debug)]
pub struct MsgMeta {
    source: AccountId,
    incl_epoch: u32,
    value: BitcoinAmount,
}

impl MsgMeta {
    pub fn new(source: AccountId, incl_epoch: u32, value: BitcoinAmount) -> Self {
        Self {
            source,
            incl_epoch,
            value,
        }
    }

    /// Gets the ID of the account the sent the message.
    pub fn source(&self) -> AccountId {
        self.source
    }

    /// Gets the epoch that the message was included in the input queue.
    pub fn incl_epoch(&self) -> u32 {
        self.incl_epoch
    }

    /// Gets the value passed with the message (in sats).
    pub fn value(&self) -> BitcoinAmount {
        self.value
    }
}

/// Represents a parsed message.
#[derive(Clone, Debug)]
pub struct InputMessage<M: IAcctMsg> {
    meta: MsgMeta,
    decoded: Option<M>,
}

impl<M: IAcctMsg> InputMessage<M> {
    /// Creates a new [`InputMessage`] from its parts.
    pub fn new(meta: MsgMeta, decoded: Option<M>) -> Self {
        Self { meta, decoded }
    }

    /// Creates a valid [`InputMessage`] from a meta and decoded message.
    pub fn from_msg(meta: MsgMeta, msg: M) -> Self {
        Self::new(meta, Some(msg))
    }

    /// Parses from a buf with a [`MsgMeta`], hiding any error and falling back
    /// to an unknown message.
    fn from_buf_coerce(meta: MsgMeta, buf: &[u8]) -> Self {
        Self {
            meta,
            decoded: M::try_parse(buf).ok(),
        }
    }

    /// Parses an [`InputMessage`] from a [`MessageEntry`], preparing it to be
    /// consumed.
    ///
    /// This gobbles errors, because if it's a [`MessageEntry`] then we can
    /// probably assume it's already coming from an inbox or would be.
    pub fn from_msg_entry(entry: &MessageEntry) -> Self {
        let meta = MsgMeta::new(entry.source(), entry.incl_epoch(), entry.payload_value());
        Self::from_buf_coerce(meta, entry.payload_buf())
    }

    /// Checks if the message was successfully decoded.
    pub fn is_valid(&self) -> bool {
        self.decoded.is_some()
    }

    /// Gets the decoded message, if parsing succeeded.
    pub fn message(&self) -> Option<&M> {
        self.decoded.as_ref()
    }

    /// Gets the message meta.
    pub fn meta(&self) -> &MsgMeta {
        &self.meta
    }
}
