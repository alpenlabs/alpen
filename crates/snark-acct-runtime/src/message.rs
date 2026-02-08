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
// TODO does this make sense to just be a struct with the `M` as an Option?
#[derive(Clone, Debug)]
pub enum InputMessage<M: IAcctMsg> {
    Valid(MsgMeta, M),
    Unknown(MsgMeta),
}

impl<M: IAcctMsg> InputMessage<M> {
    /// Parses from a buf with a [`MsgMeta`], hiding any error and falling back
    /// to `Unknown`.
    fn from_buf_coerce(meta: MsgMeta, buf: &[u8]) -> Self {
        match M::try_parse(buf) {
            Ok(m) => Self::Valid(meta, m),
            Err(_) => Self::Unknown(meta),
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

    /// Checks if the message is value.
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_, _))
    }

    /// Gets the message, if we parsed it correctly.
    pub fn message(&self) -> Option<&M> {
        match self {
            Self::Valid(_, m) => Some(m),
            _ => None,
        }
    }

    /// Gets the message meta.
    pub fn meta(&self) -> &MsgMeta {
        match self {
            Self::Valid(m, _) => m,
            Self::Unknown(m) => m,
        }
    }
}
