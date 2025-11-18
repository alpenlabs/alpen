use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::AccountId;
use strata_snark_acct_types::SnarkAccountUpdateContainer;

use crate::Slot;

/// Represents a single transaction within a block.
#[derive(Clone, Debug)]
pub struct OLTransaction {
    /// Any extra data associated with the transaction.
    extra: TransactionAttachment,

    /// The actual payload for the transaction.
    payload: TransactionPayload,
}

impl OLTransaction {
    // TODO use a builder
    pub(crate) fn new(extra: TransactionAttachment, payload: TransactionPayload) -> Self {
        Self { payload, extra }
    }

    pub fn attachments(&self) -> &TransactionAttachment {
        &self.extra
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.payload
    }

    pub fn type_id(&self) -> TxTypeId {
        self.payload().type_id()
    }
}

/// The actual payload of the transaction.
// TODO probably convert these from being struct-like variants
#[derive(Clone, Debug)]
pub enum TransactionPayload {
    GenericAccountMessage(GamTxPayload),
    SnarkAccountUpdate(SnarkAccountUpdateTxPayload),
}

impl TransactionPayload {
    pub fn type_id(&self) -> TxTypeId {
        match self {
            TransactionPayload::GenericAccountMessage { .. } => TxTypeId::GenericAccountMessage,
            TransactionPayload::SnarkAccountUpdate { .. } => TxTypeId::SnarkAccountUpdate,
        }
    }
}

/// Additional constraints that we can place on a transaction.
///
/// This isn't *that* useful for now, but will be in the future.
#[derive(Clone, Debug, Default)]
pub struct TransactionAttachment {
    min_slot: Option<Slot>,
    max_slot: Option<Slot>,
}

impl TransactionAttachment {
    pub fn new_empty() -> Self {
        Self {
            min_slot: None,
            max_slot: None,
        }
    }

    pub fn min_slot(&self) -> Option<Slot> {
        self.min_slot
    }

    pub fn set_min_slot(&mut self, min_slot: Option<Slot>) {
        self.min_slot = min_slot;
    }

    pub fn max_slot(&self) -> Option<Slot> {
        self.max_slot
    }

    pub fn set_max_slot(&mut self, max_slot: Option<Slot>) {
        self.max_slot = max_slot;
    }
}

/// Type ID to indicate transaction types.
#[repr(u16)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, IntEnum)]
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
        f.write_str(s)
    }
}

/// "Generic Account Message" tx payload.
#[derive(Clone, Debug)]
pub struct GamTxPayload {
    target: AccountId,
    payload: Vec<u8>,
}

impl GamTxPayload {
    pub fn new(target: AccountId, payload: Vec<u8>) -> Self {
        Self { target, payload }
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Snark account update payload.
#[derive(Clone, Debug)]
pub struct SnarkAccountUpdateTxPayload {
    target: AccountId,
    update_container: SnarkAccountUpdateContainer,
}

impl SnarkAccountUpdateTxPayload {
    pub fn new(target: AccountId, update_container: SnarkAccountUpdateContainer) -> Self {
        Self {
            target,
            update_container,
        }
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn update_container(&self) -> &SnarkAccountUpdateContainer {
        &self.update_container
    }
}
