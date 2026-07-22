//! Abstract description of an update.

use strata_acct_types::{AccountId, AccumulatorClaim, ITxEffects, MessageEntry, MsgPayload};
use strata_identifiers::{Buf32, Epoch};

use crate::Seqno;

/// Abstractly describes a snark account update.
pub trait ISnarkAccountUpdateData {
    type Message: IMessageEntry;
    type LedgerRefs: ILedgerRefs;
    type Effects: ITxEffects;

    /// Sequence number.
    fn seq_no(&self) -> Seqno;

    /// The new inner state being updated to.
    fn new_inner_state(&self) -> Buf32;

    /// The new index of the next message to be processed.
    fn new_next_msg_idx(&self) -> u64;

    /// The number of messgaes processed by the update.
    fn num_messasges(&self) -> u64;

    /// Returns an iterator over the messages processed by the update.
    fn messages_iter(&self) -> impl Iterator<Item = Self::Message>;

    /// SAU extra data persisted to L1.
    ///
    /// This MUST be within bounds.
    fn extra_data(&self) -> &[u8];

    /// Gets the ledger refs we made.
    fn ledger_refs(&self) -> Self::LedgerRefs;

    /// Gets the effects of the transaction.
    fn effects(&self) -> Self::Effects;
}

/// Decsribes a message entry checked against the inbox accumulator by the update.
pub trait IMessageEntry {
    /// The source account ID of the message.
    fn source(&self) -> AccountId;

    /// The epoch that this message was sent.
    fn incl_epoch(&self) -> Epoch;

    /// Gets a clone of the underlying payload.
    fn get_payload(&self) -> MsgPayload;

    /// Computes the commitment stored in the inbox MMR accumulator.
    fn compute_commitment(&self) -> Buf32 {
        MessageEntry::new(self.source(), self.incl_epoch(), self.get_payload())
            .compute_msg_commitment()
    }
}

/// Container for the refs to other accumulators in the ledger we make.
pub trait ILedgerRefs {
    /// Gets the number of L1 block refs made.
    fn num_l1_block_refs(&self) -> usize;

    /// Gets an L1 block ref.
    fn get_l1_block_ref(&self, idx: usize) -> Option<AccumulatorClaim>;

    /// Returns an iterator over the L1 block refs.
    fn l1_block_refs_iter(&self) -> impl Iterator<Item = AccumulatorClaim> {
        (0..self.num_l1_block_refs()).map(|i| {
            self.get_l1_block_ref(i)
                .expect("snark-acct: incorrect ILedgerRefs impl")
        })
    }
}
