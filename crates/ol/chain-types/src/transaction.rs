use strata_acct_types::AccountId;
use strata_snark_acct_types::SnarkAccountUpdate;

use crate::Slot;

pub type TxTypeId = u16;

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

    pub fn target(&self) -> AccountId {
        self.payload().target()
    }

    pub fn type_id(&self) -> TxTypeId {
        match self.payload {
            TransactionPayload::GenericAccountMessage { .. } => 1,
            TransactionPayload::SnarkAccountUpdate { .. } => 2,
        }
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
    pub fn target(&self) -> AccountId {
        match self {
            TransactionPayload::SnarkAccountUpdate { target, .. } => *target,
            TransactionPayload::GenericAccountMessage { target, .. } => *target,
        }
    }
}

/// Additional data in a transaction.
#[derive(Clone, Debug)]
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
