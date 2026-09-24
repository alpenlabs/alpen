//! Base state layer over a fully materialized chainstate.

use std::collections::BTreeMap;

use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, L1BlockRecord, Mmr64};
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ol_params::OLParams;
use strata_ol_state_container::{OLStateContainer, OLStateSeries};
use strata_ol_state_types::*;
use strata_ol_state_types_v1::{IStateBatchApplicable, OLAccountStateV1, OLStateV1, WriteBatch};

use crate::write_tracking_layer::IComputeStateRootWithWrites;

/// Base layer holding a fully materialized chainstate in memory, together with
/// the spec versions of its [`OLRootState`].
///
/// The layer is generic over the chainstate layout `S` and implements the
/// state accessor traits once per layout, so rules code can use the concrete
/// account types of that layout. [`OLStateV1`] is the only layout today.
///
/// The layer never stores `chainstate_root`. [`IStateAccessor::compute_state_root`]
/// recomputes it from the chainstate, and [`Self::into_container`] computes it
/// once, so no mutation can leave a stale root behind. States enter and leave
/// the layer as [`OLStateContainer`]s, which keeps the spec versions; only
/// [`Self::new_genesis`] assigns versions.
#[derive(Clone, Debug)]
pub struct MemoryStateBaseLayer<S> {
    /// Spec the chainstate was produced under.
    ///
    /// Always a spec whose layout is `S`.
    cur_spec: OLSpecId,

    /// Raw spec version the next epoch runs under, which may name a spec this
    /// binary does not know.
    staged_spec_version: u32,

    /// The fully-materialized chainstate in memory.
    ///
    /// This includes the transitional embedded accounts table.
    chainstate: S,

    /// Stored lookup table of account serials to account IDs so we don't have
    /// to traverse the accounts list.
    serials: BTreeMap<AccountSerial, AccountId>,
}

impl MemoryStateBaseLayer<OLStateV1> {
    /// Constructs the genesis layer for `params`: the genesis chainstate, with
    /// both spec versions at [`OLSpecId::GENESIS`].
    ///
    /// This is the only constructor that assigns versions, and it only accepts
    /// genesis parameters, so no other state can be given genesis versions by
    /// accident. Every other state must be built with [`Self::from_container`]
    /// so it carries its own versions.
    pub fn new_genesis(params: &OLParams) -> StateResult<Self> {
        let chainstate = OLStateV1::from_genesis_params(params)?;
        Ok(Self::from_parts(
            OLSpecId::GENESIS,
            OLSpecId::GENESIS.into(),
            chainstate,
        ))
    }

    /// Constructs a layer from a container, keeping its spec versions.
    ///
    /// # Panics
    ///
    /// If the state's accounts have duplicated serials.
    pub fn from_container(container: OLStateContainer) -> Self {
        let cur_spec = container.cur_spec();
        let staged_spec_version = container.staged_spec_version();
        let (_, chainstate) = container.into_parts();
        let OLStateSeries::V1(chainstate) = chainstate;
        Self::from_parts(cur_spec, staged_spec_version, chainstate)
    }

    /// Indexes the serials of `chainstate`.
    fn from_parts(cur_spec: OLSpecId, staged_spec_version: u32, chainstate: OLStateV1) -> Self {
        let serials: BTreeMap<_, _> = chainstate
            .ledger
            .accounts
            .iter()
            .map(|a| (a.state.serial, a.id))
            .collect();

        assert_eq!(
            serials.len(),
            chainstate.ledger.accounts.len(),
            "ol/state-support: state has duplicated serials"
        );

        Self {
            cur_spec,
            staged_spec_version,
            chainstate,
            serials,
        }
    }

    /// Converts the layer into a container, computing the chainstate root.
    pub fn into_container(self) -> OLStateContainer {
        OLStateContainer::new(
            self.cur_spec,
            self.staged_spec_version,
            OLStateSeries::V1(self.chainstate),
        )
    }

    /// Builds a container from a copy of the layer's state, computing the
    /// chainstate root.
    pub fn to_container(&self) -> OLStateContainer {
        OLStateContainer::new(
            self.cur_spec,
            self.staged_spec_version,
            OLStateSeries::V1(self.chainstate.clone()),
        )
    }

    /// Returns the chainstate.
    pub fn chainstate(&self) -> &OLStateV1 {
        &self.chainstate
    }

    /// Computes the protocol state root for `chainstate` under this layer's
    /// spec versions.
    fn compute_root_for(&self, chainstate: &OLStateV1) -> Buf32 {
        OLRootState::new(
            self.cur_spec.into(),
            self.staged_spec_version,
            chainstate.compute_chainstate_root(),
        )
        .compute_state_root()
    }
}

impl IStateAccessor for MemoryStateBaseLayer<OLStateV1> {
    type AccountState = OLAccountStateV1;

    // ===== Root state methods =====

    fn cur_spec_version(&self) -> u32 {
        self.cur_spec.into()
    }

    fn staged_spec_version(&self) -> u32 {
        self.staged_spec_version
    }

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.chainstate.global.get_cur_slot()
    }

    fn limbo_funds(&self) -> BitcoinAmount {
        self.chainstate.global.limbo_funds()
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.chainstate.epoch.cur_epoch()
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.chainstate.epoch.last_l1_blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        self.chainstate.epoch.last_l1_height()
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.chainstate.epoch.asm_recorded_epoch()
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.chainstate.epoch.total_ledger_balance()
    }

    fn l1_block_refs_mmr(&self) -> &Mmr64 {
        self.chainstate.epoch.l1_block_refs_mmr()
    }

    // ===== Intraepoch state methods =====

    fn pending_asm_logs_len(&self) -> usize {
        self.chainstate.intraepoch_state().pending_asm_logs().len()
    }

    fn get_pending_asm_log(&self, idx: usize) -> Option<PendingAsmLog> {
        self.chainstate
            .intraepoch_state()
            .pending_asm_logs()
            .get(idx)
            .map(PendingAsmLog::from)
    }

    fn pending_asm_logs_full(&self) -> bool {
        self.chainstate.intraepoch_state().is_pending_logs_full()
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> StateResult<bool> {
        Ok(self.chainstate.ledger.get_account_state(&id).is_some())
    }

    fn get_account_state(&self, id: AccountId) -> StateResult<Option<&Self::AccountState>> {
        Ok(self.chainstate.ledger.get_account_state(&id))
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> StateResult<Option<AccountId>> {
        Ok(self.serials.get(&serial).copied())
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.chainstate.global.get_next_avail_serial()
    }

    fn compute_state_root(&self) -> StateResult<Buf32> {
        Ok(self.compute_root_for(&self.chainstate))
    }
}

impl IStateAccessorMut for MemoryStateBaseLayer<OLStateV1> {
    type AccountStateMut = OLAccountStateV1;

    fn set_cur_slot(&mut self, slot: u64) {
        self.chainstate.global.set_cur_slot(slot);
    }

    fn add_limbo_funds_coin(&mut self, coin: Coin) -> StateResult<()> {
        let cur = self.chainstate.global.limbo_funds();
        let amt = coin.amt();
        let new_limbo_funds = cur
            .to_sat()
            .checked_add(amt.to_sat())
            .and_then(|sats| BitcoinAmount::try_from(sats).ok());
        if new_limbo_funds.is_none() {
            // Defuse the coin before returning: the whole STF is discarded on
            // this error, so no value is actually lost, and dropping a live coin
            // would panic in `Coin::drop`.
            coin.safely_consume_unchecked();
            return Err(StateError::LimboFundsOverflow { cur, add: amt });
        }
        self.chainstate.global.add_limbo_funds_coin(coin);
        Ok(())
    }

    fn take_limbo_funds_coin(&mut self, amt: BitcoinAmount) -> StateResult<Coin> {
        self.chainstate.global.take_limbo_funds_coin(amt).ok_or(
            StateError::InsufficientLimboFunds {
                need: amt,
                have: self.chainstate.global.limbo_funds(),
            },
        )
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.chainstate.epoch.set_cur_epoch(epoch);
    }

    fn append_l1_block_rec(&mut self, height: L1Height, rec: L1BlockRecord) {
        self.chainstate.epoch.append_l1_block_rec(height, rec);
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.chainstate.epoch.set_asm_recorded_epoch(epoch);
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.chainstate.epoch.set_total_ledger_balance(amt);
    }

    fn try_append_pending_asm_log(&mut self, entry: PendingAsmLog) -> StateResult<()> {
        let ssz_entry = entry.into();
        self.chainstate
            .intraepoch_state_mut()
            .try_append_pending_log(ssz_entry)
    }

    fn reset_intraepoch_state(&mut self) {
        self.chainstate.intraepoch_state_mut().reset();
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> StateResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        let acct = self
            .chainstate
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
        let serial = self.chainstate.global.get_next_avail_serial();
        self.chainstate
            .create_new_account(id, serial, new_acct_data)?;
        self.serials.insert(serial, id);
        Ok(serial)
    }
}

impl IStateBatchApplicable for MemoryStateBaseLayer<OLStateV1> {
    fn apply_write_batch(&mut self, batch: WriteBatch) -> StateResult<()> {
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

        self.chainstate.apply_write_batch(batch)?;

        for (serial, id) in new_accounts {
            self.serials.insert(serial, id);
        }

        Ok(())
    }
}

impl IComputeStateRootWithWrites for MemoryStateBaseLayer<OLStateV1> {
    fn compute_state_root_with_writes<'b>(
        &self,
        writes: impl Iterator<Item = &'b WriteBatch>,
    ) -> StateResult<Buf32> {
        let mut chainstate = self.chainstate.clone();

        for wb in writes {
            // Maybe we can avoid this clone?
            chainstate.apply_write_batch(wb.clone())?;
        }

        // Write batches carry no spec versions, so the root keeps this
        // layer's versions.
        // TODO(STR-4086): apply the batches' version writes once `WriteBatch`
        // carries them, or overlay roots will miss staged and promoted specs.
        Ok(self.compute_root_for(&chainstate))
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_ol_state_types_v1::{IStateBatchApplicable, WriteBatch};

    use super::*;
    use crate::common_tests::{impl_mut_layer_tests, impl_read_layer_tests};
    use crate::test_utils::*;

    /// Builds the layer under test as a clone of the fixture base.
    ///
    /// The base layer owns its state outright, so unlike the wrapper layers it
    /// has to be cloned to be written to. That makes the suite's "the base is
    /// untouched" assertions vacuous for this stack; everything else still pins
    /// the reference semantics the wrappers are expected to match.
    macro_rules! build_base_clone {
        ($base:expr, $layer:ident) => {
            let $layer = $base.clone();
        };
        ($base:expr, mut $layer:ident) => {
            let mut $layer = $base.clone();
        };
    }

    impl_read_layer_tests!(build_base_clone);
    impl_mut_layer_tests!(build_base_clone);

    /// Applies a write batch that creates a new account and confirms the
    /// freshly-allocated serial is reachable via `find_account_id_by_serial`.
    #[test]
    fn test_apply_write_batch_indexes_new_account_serials() {
        let mut layer = create_test_base_layer();

        let account_id = test_account_id(7);
        let serial = layer.next_account_serial();

        let snark_state = test_snark_account_state(7);
        let new_acct = test_new_snark_account_data(
            &snark_state,
            BitcoinAmount::try_from(1_234)
                .expect("amount must not exceed the Bitcoin money supply"),
        );

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
