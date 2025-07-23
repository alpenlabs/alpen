//! Deposit-related types and tables.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, OutputRef},
};

/// Container for the state machine of a deposit factory.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DepositEntry {
    deposit_idx: u32,

    /// The outpoint that this deposit entry references.
    output: OutputRef,

    /// List of notary operators, by their indexes.
    // TODO convert this to a windowed bitmap or something
    notary_operators: Vec<OperatorIdx>,

    /// Deposit amount, in the native asset.
    amt: BitcoinAmount,
}

impl DepositEntry {
    pub fn new(
        idx: u32,
        output: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> Self {
        Self {
            deposit_idx: idx,
            output,
            notary_operators: operators,
            amt,
        }
    }

    pub fn idx(&self) -> u32 {
        self.deposit_idx
    }

    pub fn output(&self) -> &OutputRef {
        &self.output
    }

    pub fn notary_operators(&self) -> &[OperatorIdx] {
        &self.notary_operators
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct DepositsTable {
    /// Next unassigned deposit index.
    next_idx: u32,

    /// Deposit table.
    ///
    /// MUST be sorted by `deposit_idx`.
    deposits: Vec<DepositEntry>,
}

impl DepositsTable {
    pub fn new_empty() -> Self {
        Self {
            next_idx: 0,
            deposits: Vec::new(),
        }
    }

    /// Returns the number of deposit entries being tracked.
    pub fn len(&self) -> u32 {
        self.deposits.len() as u32
    }

    /// Returns if the deposit table is empty.  This is practically probably
    /// never going to be true.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Gets the position in the deposit table of a hypothetical deposit entry
    /// index.
    pub fn get_deposit_entry_pos(&self, idx: u32) -> Result<u32, u32> {
        self.deposits
            .binary_search_by_key(&idx, |e| e.deposit_idx)
            .map(|i| i as u32)
            .map_err(|i| i as u32)
    }

    /// Gets a deposit from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_deposit(&self, idx: u32) -> Option<&DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &self.deposits[i as usize])
    }

    /// Gets a mut ref to a deposit from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_deposit_mut(&mut self, idx: u32) -> Option<&mut DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &mut self.deposits[i as usize])
    }

    pub fn get_all_deposits_idxs_iters_iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.deposits.iter().map(|e| e.deposit_idx)
    }

    /// Gets a deposit entry by its internal position, *ignoring* the indexes.
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&DepositEntry> {
        self.deposits.get(pos as usize)
    }

    /// Adds a new deposit to the table and returns the index of the new deposit.
    pub fn create_next_deposit(
        &mut self,
        tx_ref: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> u32 {
        let idx = self.next_idx();
        let deposit_entry = DepositEntry::new(idx, tx_ref, operators, amt);
        self.deposits.push(deposit_entry);
        self.next_idx += 1;
        idx
    }

    /// Tries to create a deposit entry at a specific idx.  If the entry requested if after the
    /// `next_entry`, then updates it to be equal to that.
    ///
    /// Returns if we inserted it successfully.
    pub fn try_create_deposit_at(
        &mut self,
        idx: u32,
        tx_ref: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> bool {
        // Happy case, if we're creating the next entry we can skip the binary
        // search.  This should be most cases, where there isn't concurrent
        // interleaved deposit processing.
        if idx == self.next_idx {
            self.create_next_deposit(tx_ref, operators, amt);
            return true;
        }

        // Slow path.
        match self.get_deposit_entry_pos(idx) {
            Ok(_) => false,
            Err(pos) => {
                let entry = DepositEntry::new(idx, tx_ref, operators, amt);
                self.deposits.insert(pos as usize, entry);

                // Tricky bookkeeping.
                if idx >= self.next_idx {
                    self.next_idx = u32::max(self.next_idx, idx) + 1;
                }

                true
            }
        }
    }

    pub fn next_idx(&self) -> u32 {
        self.next_idx
    }

    pub fn deposits(&self) -> impl Iterator<Item = &DepositEntry> {
        self.deposits.iter()
    }
}
