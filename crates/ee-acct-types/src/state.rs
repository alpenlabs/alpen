//! EE account internal state.

use strata_acct_types::BitcoinAmount;
use strata_identifiers::Hash;
use strata_snark_acct_runtime::IInnerState;

use crate::ssz_generated::ssz::state::{EeAccountState, PendingFinclEntry, PendingInputEntry};

impl EeAccountState {
    pub fn new(
        last_exec_blkid: Hash,
        tracked_balance: BitcoinAmount,
        pending_inputs: Vec<PendingInputEntry>,
        pending_fincls: Vec<PendingFinclEntry>,
    ) -> Self {
        Self {
            last_exec_blkid: last_exec_blkid.0.into(),
            tracked_balance,
            pending_inputs: pending_inputs.into(),
            pending_fincls: pending_fincls.into(),
        }
    }

    pub fn into_parts(
        self,
    ) -> (
        Hash,
        BitcoinAmount,
        Vec<PendingInputEntry>,
        Vec<PendingFinclEntry>,
    ) {
        (
            self.last_exec_blkid
                .as_ref()
                .try_into()
                .expect("FixedBytes<32> should convert to [u8; 32]"),
            self.tracked_balance,
            self.pending_inputs.into(),
            self.pending_fincls.into(),
        )
    }

    pub fn last_exec_blkid(&self) -> Hash {
        self.last_exec_blkid
            .as_ref()
            .try_into()
            .expect("FixedBytes<32> should convert to [u8; 32]")
    }

    pub fn set_last_exec_blkid(&mut self, blkid: Hash) {
        self.last_exec_blkid = blkid.0.into();
    }

    pub fn tracked_balance(&self) -> BitcoinAmount {
        self.tracked_balance
    }

    /// Adds to the tracked balance, panicking on overflow.
    pub fn add_tracked_balance(&mut self, amt: BitcoinAmount) {
        self.tracked_balance = self
            .tracked_balance
            .checked_add(amt)
            .expect("snarkacct: overflowing balance");
    }

    pub fn pending_inputs(&self) -> &[PendingInputEntry] {
        &self.pending_inputs
    }

    pub fn add_pending_input(&mut self, inp: PendingInputEntry) -> bool {
        self.pending_inputs.push(inp).is_ok()
    }

    /// Removing some number of pending inputs.
    pub fn remove_pending_inputs(&mut self, n: usize) -> bool {
        if self.pending_inputs.len() < n {
            false
        } else {
            let mut vec: Vec<_> = self.pending_inputs.clone().into();
            vec.drain(..n);
            self.pending_inputs = vec.into();
            true
        }
    }

    pub fn pending_fincls(&self) -> &[PendingFinclEntry] {
        &self.pending_fincls
    }

    pub fn add_pending_fincl(&mut self, inp: PendingFinclEntry) -> bool {
        self.pending_fincls.push(inp).is_ok()
    }

    /// Removing some number of pending forced inclusions.
    pub fn remove_pending_fincls(&mut self, n: usize) -> bool {
        if self.pending_fincls.len() < n {
            false
        } else {
            let mut vec: Vec<_> = self.pending_fincls.clone().into();
            vec.drain(..n);
            self.pending_fincls = vec.into();
            true
        }
    }
}

impl IInnerState for EeAccountState {
    fn compute_state_root(&self) -> Hash {
        // TODO: Use SSZ hash_tree_root when available.
        Hash::default()
    }
}

impl PendingInputEntry {
    pub fn ty(&self) -> PendingInputType {
        match self {
            PendingInputEntry::Deposit(_) => PendingInputType::Deposit,
        }
    }
}

/// Pending input type.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum PendingInputType {
    Deposit,
}

impl PendingFinclEntry {
    pub fn new(epoch: u32, raw_tx_hash: Hash) -> Self {
        Self {
            epoch,
            raw_tx_hash: raw_tx_hash.0.into(),
        }
    }

    pub fn into_parts(self) -> (u32, Hash) {
        (
            self.epoch,
            self.raw_tx_hash
                .as_ref()
                .try_into()
                .expect("FixedBytes<32> should convert to [u8; 32]"),
        )
    }

    pub fn epoch(&self) -> &u32 {
        &self.epoch
    }

    pub fn raw_tx_hash(&self) -> Hash {
        self.raw_tx_hash
            .as_ref()
            .try_into()
            .expect("FixedBytes<32> should convert to [u8; 32]")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_acct_types::{BitcoinAmount, SubjectId};
    use strata_ee_chain_types::SubjectDepositData;
    use strata_test_utils_ssz::ssz_proptest;

    use crate::ssz_generated::ssz::state::{EeAccountState, PendingFinclEntry, PendingInputEntry};

    fn subject_deposit_data_strategy() -> impl Strategy<Value = SubjectDepositData> {
        (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest_bytes, value)| SubjectDepositData {
            dest: SubjectId::from(dest_bytes),
            value: BitcoinAmount::from_sat(value),
        })
    }

    fn pending_input_entry_strategy() -> impl Strategy<Value = PendingInputEntry> {
        subject_deposit_data_strategy().prop_map(PendingInputEntry::Deposit)
    }

    mod pending_input_entry {
        use super::*;

        ssz_proptest!(PendingInputEntry, pending_input_entry_strategy());
    }

    mod pending_fincl_entry {
        use super::*;

        ssz_proptest!(
            PendingFinclEntry,
            (any::<u32>(), any::<[u8; 32]>()).prop_map(|(epoch, hash)| PendingFinclEntry {
                epoch,
                raw_tx_hash: hash.into(),
            })
        );
    }

    mod ee_account_state {
        use super::*;

        ssz_proptest!(
            EeAccountState,
            (
                any::<[u8; 32]>(),
                any::<u64>(),
                prop::collection::vec(pending_input_entry_strategy(), 0..5),
                prop::collection::vec(
                    (any::<u32>(), any::<[u8; 32]>()).prop_map(|(epoch, hash)| PendingFinclEntry {
                        epoch,
                        raw_tx_hash: hash.into(),
                    }),
                    0..5,
                ),
            )
                .prop_map(|(last_exec_blkid, balance, inputs, fincls)| {
                    EeAccountState {
                        last_exec_blkid: last_exec_blkid.into(),
                        tracked_balance: BitcoinAmount::from_sat(balance),
                        pending_inputs: inputs.into(),
                        pending_fincls: fincls.into(),
                    }
                })
        );
    }
}
