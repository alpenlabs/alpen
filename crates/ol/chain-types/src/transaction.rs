use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::AccountId;
use strata_primitives::Slot;
use strata_snark_acct_types::SnarkAccountUpdateWithMmrProofs;

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
#[expect(clippy::large_enum_variant, reason = "..")]
#[derive(Clone, Debug)]
pub enum TransactionPayload {
    GenericAccountMessage {
        target: AccountId,
        payload: Vec<u8>,
    },
    SnarkAccountUpdate {
        target: AccountId,
        update: SnarkAccountUpdateWithMmrProofs,
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
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, IntEnum)]
pub enum TxTypeId {
    /// Transactions that are messages being sent to other accounts.
    GenericAccountMessage = 1,

    /// Transactions that are snark account updates.
    SnarkAccountUpdate = 2,
}

impl fmt::Display for TxTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TxTypeId::SnarkAccountUpdate => "snark-account-update",
            TxTypeId::GenericAccountMessage => "generic-account-message",
        };
        write!(f, "{}", s)
    }
}
