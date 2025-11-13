use std::fmt;

use digest::Digest;
use int_enum::IntEnum;
use sha2::Sha256;
use strata_acct_types::AccountId;
use strata_identifiers::OLTxId;
use strata_primitives::buf::Buf32;
use strata_snark_acct_types::SnarkAccountUpdate;

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

    /// Compute the transaction ID (hash of transaction contents).
    ///
    /// The txid is computed by hashing all transaction fields in a deterministic order.
    /// This enables duplicate detection and transaction identification in the mempool.
    ///
    /// TODO: Replace with canonical encoding (SSZ or Borsh) once transaction types
    /// have Serialize derives.
    pub fn compute_txid(&self) -> OLTxId {
        let mut hasher = Sha256::new();

        // Hash transaction type
        hasher.update([self.type_id() as u8]);

        // Hash target account
        if let Some(target) = self.target() {
            hasher.update(target.inner());
        }

        // Hash payload data (type-specific)
        match &self.payload {
            TransactionPayload::GenericAccountMessage { payload, .. } => {
                hasher.update((payload.len() as u64).to_le_bytes());
                hasher.update(payload);
            }
            TransactionPayload::SnarkAccountUpdate { update, .. } => {
                // Hash key fields of the update
                hasher.update(update.operation().seq_no().to_le_bytes());
                hasher.update(update.operation().new_state().inner_state());

                // Hash number of messages processed
                hasher.update((update.operation().processed_messages().len() as u64).to_le_bytes());

                // Hash update proof
                hasher.update((update.update_proof().len() as u64).to_le_bytes());
                hasher.update(update.update_proof());
            }
        }

        // Hash extra fields
        if let Some(min_slot) = self.extra.min_slot() {
            hasher.update([1u8]); // Presence marker
            hasher.update(min_slot.to_le_bytes());
        } else {
            hasher.update([0u8]);
        }

        if let Some(max_slot) = self.extra.max_slot() {
            hasher.update([1u8]); // Presence marker
            hasher.update(max_slot.to_le_bytes());
        } else {
            hasher.update([0u8]);
        }

        OLTxId::from(Buf32::from(<[u8; 32]>::from(hasher.finalize())))
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
