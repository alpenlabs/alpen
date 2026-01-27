use strata_acct_types::{AccountId, BitcoinAmount};

use crate::traits::IAcctMsg;

/// Meta fields extracted from a message.
#[derive(Copy, Clone, Debug)]
pub struct MsgMeta {
    #[expect(dead_code, reason = "for future use")]
    pub(crate) source: AccountId,
    #[expect(dead_code, reason = "for future use")]
    pub(crate) incl_epoch: u32,
    pub(crate) value: BitcoinAmount,
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
