use strata_acct_types::AccountId;
use strata_snark_acct_types::SnarkAccountUpdate;
use thiserror::Error;

use crate::Slot;

/// Represents a single transaction within a block.
#[derive(Clone, Debug)]
pub struct OLTransaction {
    /// The actual payload for the transaction.
    payload: TransactionPayload,

    /// Any extra data associated with the transaction.
    extra: TransactionExtra,
}

impl OLTransaction {
    pub fn new(payload: TransactionPayload, extra: TransactionExtra) -> Self {
        Self { payload, extra }
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.payload
    }

    pub fn extra(&self) -> &TransactionExtra {
        &self.extra
    }

    pub fn target(&self) -> Option<AccountId> {
        self.payload().target()
    }

    pub fn type_id(&self) -> TxTypeId {
        self.payload().type_id()
    }
}

/// The actual payload of the transaction.
#[derive(Clone, Debug)]
pub enum TransactionPayload {
    GenericAccountMessage {
        target: AccountId,
        payload: Vec<u8>,
    },
    SnarkAccountUpdate {
        target: AccountId,
        update: SnarkAccountUpdate,
    },
}

impl TransactionPayload {
    pub fn target(&self) -> Option<AccountId> {
        match self {
            TransactionPayload::SnarkAccountUpdate { target, .. } => Some(*target),
            TransactionPayload::GenericAccountMessage { target, .. } => Some(*target),
        }
    }

    pub fn type_id(&self) -> TxTypeId {
        match self {
            TransactionPayload::GenericAccountMessage { .. } => TxTypeId::GenericAccountMessage,
            TransactionPayload::SnarkAccountUpdate { .. } => TxTypeId::SnarkAccountUpdate,
        }
    }
}

/// Additional data in a transaction.
#[derive(Clone, Debug, Default)]
pub struct TransactionExtra {
    min_slot: Option<Slot>,
    max_slot: Option<Slot>,
}

impl TransactionExtra {
    pub fn new(min_slot: Option<Slot>, max_slot: Option<Slot>) -> Self {
        Self { min_slot, max_slot }
    }

    pub fn min_slot(&self) -> Option<Slot> {
        self.min_slot
    }

    pub fn max_slot(&self) -> Option<Slot> {
        self.max_slot
    }
}

/// A type-safe representation of transaction type id.
#[repr(u16)]
#[derive(Debug, Copy, Clone)]
pub enum TxTypeId {
    GenericAccountMessage = 1,
    SnarkAccountUpdate = 2,
}

impl From<TxTypeId> for u16 {
    #[inline]
    fn from(value: TxTypeId) -> Self {
        value as u16
    }
}

impl TryFrom<u16> for TxTypeId {
    type Error = TxTypeError;

    #[inline]
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(TxTypeId::GenericAccountMessage),
            2 => Ok(TxTypeId::SnarkAccountUpdate),
            _ => Err(TxTypeError::InvalidTxType(v)),
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum TxTypeError {
    #[error("Invalid tx-type value: {0}")]
    InvalidTxType(u16),
}
