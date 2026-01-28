use strata_acct_types::{AccountId, BitcoinAmount};

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
pub enum InputMessage<M: IAcctMsg> {
    Valid(MsgMeta, M),
    Unknown(MsgMeta),
}

impl<M: IAcctMsg> InputMessage<M> {
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_, _))
    }

    pub fn message(&self) -> Option<&M> {
        match self {
            Self::Valid(_, m) => Some(m),
            _ => None,
        }
    }

    pub fn meta(&self) -> &MsgMeta {
        match self {
            Self::Valid(m, _) => m,
            Self::Unknown(m) => m,
        }
    }
}
