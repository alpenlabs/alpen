use strata_acct_types::AccountId;
use strata_snark_acct_types::SnarkAccountUpdate;

use crate::block::Slot;

/// Represents a single transaction within a block.
#[derive(Debug, Clone)]
pub struct OLTransaction {
    // TODO: type_id? I am not sure how this is going to play out with SSZ. The type id should
    // corresponding to some enum variant which is strictly typed. We won't be working with raw
    // tx payload bytes, they must be decoded into some concrete type.
    // ..
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

    /// The account id this transaction is on behalf of. `target` is confusing.
    /// Maybe we could also store sequencer pubkey
    /// along with vk? and then we can have transactions to update the pubkey if sequencer needs to
    /// rotate. Just a thought.
    pub fn account_id(&self) -> AccountId {
        match self.payload() {
            TransactionPayload::SnarkAccountUpdate { target, .. } => *target,
            // FIXME: this is probably not correct for Generic Account Message
            TransactionPayload::GenericAccountMessage { target, .. } => *target,
        }
    }
}

/// The actual payload of the transaction.
#[derive(Debug, Clone)]
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

/// Additional data in a transaction.
#[derive(Debug, Clone)]
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
