use strata_acct_types::{AccountId, AccumulatorClaim, BitcoinAmount, MessageEntry};
use strata_identifiers::Buf32;

use super::{object::IChainObj, transaction::ITargetTx};

/// Snark account update transaction.
///
/// This indicates the target account of the update (in [`ITargetTx`]) and the
/// operation we're performing to the snark account.
pub trait ISauTransaction: IChainObj + ITargetTx {
    type Operation: ISauOperationData;

    /// Gets the operation data.
    fn operation(&self) -> Self::Operation;
}

/// Information about the operation.
///
/// This is the state change within the account, the messages processed, and the
/// chain accumulator checks we have to assert.
pub trait ISauOperationData {
    type Data: ISauUpdateData;
    type Message: ISauMessageEntry;
    type LedgerRefs: ISauLedgerRefs;

    /// Gets the update data.
    fn update_data(&self) -> Self::Data;

    /// Returns an iterator over the messages being processed in the update.
    fn iter_messages(&self) -> impl Iterator<Item = Self::Message>;

    /// Gets the ledger refs.
    fn ledger_refs(&self) -> Self::LedgerRefs;
}

pub trait ISauUpdateData {
    /// Sequence number of the update tx.
    fn seq_no(&self) -> u64;

    /// The new "next processed message" index after applying the update.
    fn new_next_msg_idx(&self) -> u64;

    /// The new inner state root after applying the update.
    fn new_inner_state_root(&self) -> Buf32;

    /// SAU extra data persisted to L1.
    ///
    /// This MUST be within bounds.
    fn extra_data(&self) -> &[u8];
}

pub trait ISauMessageEntry {
    /// Gets the account that sent the message.
    fn source(&self) -> AccountId;

    /// Gets the amount transferred.
    fn amount(&self) -> BitcoinAmount;

    /// Gets the message payload.
    ///
    /// The returned slice MUST be within bounds.
    fn payload_data(&self) -> &[u8];
}

/// Temporary helper impl.
impl ISauMessageEntry for MessageEntry {
    fn source(&self) -> AccountId {
        MessageEntry::source(self)
    }

    fn amount(&self) -> BitcoinAmount {
        self.payload_value()
    }

    fn payload_data(&self) -> &[u8] {
        self.payload_buf()
    }
}

/// Temporary helper impl.
impl<'m> ISauMessageEntry for &'m MessageEntry {
    fn source(&self) -> AccountId {
        MessageEntry::source(self)
    }

    fn amount(&self) -> BitcoinAmount {
        self.payload_value()
    }

    fn payload_data(&self) -> &[u8] {
        self.payload_buf()
    }
}

pub trait ISauLedgerRefs {
    fn num_l1_block_ref_claims(&self) -> usize;
    fn get_l1_block_ref_claim(&self, idx: usize) -> Option<AccumulatorClaim>;
}
