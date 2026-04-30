//! Base state layer for [`OLState`].

use std::collections::BTreeMap;

use strata_acct_types::{
    AccountId, AccountSerial, BitcoinAmount, Mmr64,
    tree_hash::{Sha256Hasher, TreeHash},
};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::*;
use strata_ol_state_types::{IStateBatchApplicable, OLAccountState, OLState, WriteBatch};

use crate::write_tracking_layer::IComputeStateRootWithWrites;

/// Base layer wrapping [`OLState`].
#[derive(Clone, Debug)]
pub struct MemoryStateBaseLayer {
    /// The fully-materialized state in memory.
    ///
    /// This includes the transitional embedded accounts table.
    state: OLState,

    /// Stored lookup table of account serials to account IDs so we don't have
    /// to traverse the accounts list.
    serials: BTreeMap<AccountSerial, AccountId>,
}

impl MemoryStateBaseLayer {
    /// Constructs a new instance.  Indexes the serials in the process.
    ///
    /// # Panics
    ///
    /// If the state's accounts have duplicated serials.
    pub fn new(state: OLState) -> Self {
        let serials: BTreeMap<_, _> = state
            .ledger
            .accounts
            .iter()
            .map(|a| (a.state.serial, a.id))
            .collect();

        assert_eq!(
            serials.len(),
            state.ledger.accounts.len(),
            "ol/state-support: state has duplicated serials"
        );

        Self { state, serials }
    }

    pub fn state(&self) -> &OLState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut OLState {
        &mut self.state
    }

    pub fn into_inner(self) -> OLState {
        self.state
    }
}

impl IStateAccessor for MemoryStateBaseLayer {
    type AccountState = OLAccountState;

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.state.global.get_cur_slot()
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.state.epoch.cur_epoch()
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.state.epoch.last_l1_blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        self.state.epoch.last_l1_height()
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.state.epoch.asm_recorded_epoch()
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.state.epoch.total_ledger_balance()
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.state.epoch.asm_manifests_mmr()
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> StateResult<bool> {
        Ok(self.state.ledger.get_account_state(&id).is_some())
    }

    fn get_account_state(&self, id: AccountId) -> StateResult<Option<&Self::AccountState>> {
        Ok(self.state.ledger.get_account_state(&id))
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> StateResult<Option<AccountId>> {
        Ok(self.serials.get(&serial).copied())
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.state.global.get_next_avail_serial()
    }

    fn compute_state_root(&self) -> StateResult<Buf32> {
        Ok(TreeHash::<Sha256Hasher>::tree_hash_root(&self.state).into())
    }
}

impl IStateAccessorMut for MemoryStateBaseLayer {
    type AccountStateMut = OLAccountState;

    fn set_cur_slot(&mut self, slot: u64) {
        self.state.global.set_cur_slot(slot);
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.state.epoch.set_cur_epoch(epoch);
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        self.state.epoch.append_manifest(height, mf);
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.state.epoch.set_asm_recorded_epoch(epoch);
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.state.epoch.set_total_ledger_balance(amt);
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> StateResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        let acct = self
            .state
            .ledger
            .get_account_state_mut(&id)
            .ok_or(StateError::MissingAccount(id))?;
        Ok(f(acct))
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData,
    ) -> StateResult<AccountSerial> {
        let serial = self.state.global.get_next_avail_serial();
        self.state.create_new_account(id, serial, new_acct_data)?;
        self.serials.insert(serial, id);
        Ok(serial)
    }
}

impl IStateBatchApplicable for MemoryStateBaseLayer {
    fn apply_write_batch(&mut self, batch: WriteBatch<Self::AccountState>) -> StateResult<()> {
        // Validate serial bookkeeping before mutating any state so that an
        // error leaves both the inner state and the serials index untouched.
        let mut new_accounts: Vec<(AccountSerial, AccountId)> =
            Vec::with_capacity(batch.ledger().new_accounts().len());
        for (serial, id) in batch.ledger().iter_new_accounts() {
            if let Some(existing) = self.serials.get(&serial) {
                return Err(StateError::AccountExistsWithSerial {
                    serial,
                    existing: *existing,
                    new: *id,
                });
            }
            new_accounts.push((serial, *id));
        }

        self.state.apply_write_batch(batch)?;

        for (serial, id) in new_accounts {
            self.serials.insert(serial, id);
        }

        Ok(())
    }
}

impl IComputeStateRootWithWrites for MemoryStateBaseLayer {
    fn compute_state_root_with_writes<'b>(
        &self,
        writes: impl Iterator<Item = &'b WriteBatch<OLAccountState>>,
    ) -> StateResult<Buf32> {
        let mut state = self.state.clone();

        for wb in writes {
            // Maybe we can avoid this clone?
            state.apply_write_batch(wb.clone())?;
        }

        Ok(TreeHash::<Sha256Hasher>::tree_hash_root(&state).into())
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_ol_state_types::{IStateBatchApplicable, WriteBatch};

    use super::*;
    use crate::test_utils::*;

    /// Applies a write batch that creates a new account and confirms the
    /// freshly-allocated serial is reachable via `find_account_id_by_serial`.
    #[test]
    fn test_apply_write_batch_indexes_new_account_serials() {
        let mut layer = MemoryStateBaseLayer::new(create_test_genesis_state());

        let account_id = test_account_id(7);
        let serial = layer.next_account_serial();

        let snark_state = test_snark_account_state(7);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(1_234));

        let mut batch = WriteBatch::default();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        // Sanity: serial isn't indexed before applying.
        assert_eq!(layer.find_account_id_by_serial(serial).unwrap(), None);

        layer
            .apply_write_batch(batch)
            .expect("apply_write_batch failed");

        let found = layer
            .find_account_id_by_serial(serial)
            .expect("lookup should not error");
        assert_eq!(found, Some(account_id));
    }
}
