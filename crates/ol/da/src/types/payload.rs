//! Top-level DA payload types.

use std::{collections::BTreeSet, marker::PhantomData};

use strata_acct_types::{AccountId, BitcoinAmount};
use strata_codec::Codec;
use strata_da_framework::{DaError, DaWrite};
use strata_identifiers::AccountSerial;
use strata_ledger_types::{
    Coin, IAccountStateMut, ISnarkAccountState, ISnarkAccountStateMut, IStateAccessor,
    NewAccountData,
};
use strata_snark_acct_types::{MessageEntry, Seqno};

use super::{
    AccountDiff, AccountInit, DaProofState, GlobalStateDiff, LedgerDiff, SnarkAccountDiff,
};

/// Versioned OL DA payload containing the state diff.
#[derive(Debug, Codec)]
pub struct OLDaPayloadV1 {
    /// State diff for the epoch.
    pub state_diff: StateDiff,
}

impl OLDaPayloadV1 {
    /// Creates a new [`OLDaPayloadV1`] from a state diff.
    pub fn new(state_diff: StateDiff) -> Self {
        Self { state_diff }
    }
}

/// Preseal OL state diff (global + ledger).
#[derive(Debug, Default, Codec)]
pub struct StateDiff {
    /// Global state diff.
    pub global: GlobalStateDiff,

    /// Ledger state diff.
    pub ledger: LedgerDiff,
}

impl StateDiff {
    /// Creates a new [`StateDiff`] from a global state diff and ledger diff.
    pub fn new(global: GlobalStateDiff, ledger: LedgerDiff) -> Self {
        Self { global, ledger }
    }
}

/// Context required to apply OL state diffs that include ledger changes.
#[derive(Clone, Copy, Debug)]
pub struct OLApplyContext<S: IStateAccessor> {
    new_account_data: fn(&AccountInit) -> Result<NewAccountData<S::AccountState>, DaError>,
}

impl<S: IStateAccessor> OLApplyContext<S> {
    pub fn new(
        new_account_data: fn(&AccountInit) -> Result<NewAccountData<S::AccountState>, DaError>,
    ) -> Self {
        Self { new_account_data }
    }

    pub fn new_account_data(
        &self,
        init: &AccountInit,
    ) -> Result<NewAccountData<S::AccountState>, DaError> {
        (self.new_account_data)(init)
    }
}

/// Adapter for applying a state diff to a concrete state accessor.
#[derive(Debug)]
pub struct OLStateDiff<S: IStateAccessor> {
    diff: StateDiff,
    _target: PhantomData<S>,
}

impl<S: IStateAccessor> OLStateDiff<S> {
    pub fn new(diff: StateDiff) -> Self {
        Self {
            diff,
            _target: PhantomData,
        }
    }

    pub fn as_inner(&self) -> &StateDiff {
        &self.diff
    }

    pub fn into_inner(self) -> StateDiff {
        self.diff
    }
}

impl<S: IStateAccessor> Default for OLStateDiff<S> {
    fn default() -> Self {
        Self::new(StateDiff::default())
    }
}

impl<S: IStateAccessor> From<StateDiff> for OLStateDiff<S> {
    fn from(diff: StateDiff) -> Self {
        Self::new(diff)
    }
}

impl<S: IStateAccessor> From<OLStateDiff<S>> for StateDiff {
    fn from(diff: OLStateDiff<S>) -> Self {
        diff.diff
    }
}

impl<S: IStateAccessor> DaWrite for OLStateDiff<S> {
    type Target = S;
    type Context = OLApplyContext<S>;

    fn is_default(&self) -> bool {
        DaWrite::is_default(&self.diff.global) && self.diff.ledger.is_empty()
    }

    fn poll_context(&self, target: &Self::Target, context: &Self::Context) -> Result<(), DaError> {
        let pre_state_next_serial = target.next_account_serial();
        validate_ledger_entries(pre_state_next_serial, &self.diff)?;
        let mut expected_serial = pre_state_next_serial;
        for entry in self.diff.ledger.new_accounts.entries() {
            context.new_account_data(&entry.init)?;
            let exists = target
                .check_account_exists(entry.account_id)
                .map_err(|_| DaError::InsufficientContext)?;
            if exists {
                return Err(DaError::InsufficientContext);
            }
            expected_serial = expected_serial.incr();
        }

        for diff in self.diff.ledger.account_diffs.entries() {
            if diff.account_serial >= pre_state_next_serial {
                return Err(DaError::InsufficientContext);
            }
            target
                .find_account_id_by_serial(diff.account_serial)
                .map_err(|_| DaError::InsufficientContext)?
                .ok_or(DaError::InsufficientContext)?;
        }
        Ok(())
    }

    fn apply(&self, target: &mut Self::Target, context: &Self::Context) -> Result<(), DaError> {
        let mut cur_slot = target.cur_slot();
        self.diff.global.cur_slot.apply(&mut cur_slot, &())?;
        target.set_cur_slot(cur_slot);

        let pre_state_next_serial = target.next_account_serial();
        validate_ledger_entries(pre_state_next_serial, &self.diff)?;
        let mut expected_serial = pre_state_next_serial;
        for entry in self.diff.ledger.new_accounts.entries() {
            let new_acct = context.new_account_data(&entry.init)?;
            let serial = target
                .create_new_account(entry.account_id, new_acct)
                .map_err(|_| DaError::InsufficientContext)?;
            if serial != expected_serial {
                return Err(DaError::InsufficientContext);
            }
            expected_serial = expected_serial.incr();
        }

        for entry in self.diff.ledger.account_diffs.entries() {
            if entry.account_serial >= pre_state_next_serial {
                return Err(DaError::InsufficientContext);
            }
            let account_id = target
                .find_account_id_by_serial(entry.account_serial)
                .map_err(|_| DaError::InsufficientContext)?
                .ok_or(DaError::InsufficientContext)?;
            apply_account_diff(target, account_id, &entry.diff)?;
        }
        Ok(())
    }
}

fn validate_ledger_entries(
    pre_state_next_serial: AccountSerial,
    diff: &StateDiff,
) -> Result<(), DaError> {
    let mut seen_new_ids = BTreeSet::new();
    for entry in diff.ledger.new_accounts.entries() {
        if !seen_new_ids.insert(entry.account_id) {
            return Err(DaError::InsufficientContext);
        }
    }

    let pre_serial: u32 = pre_state_next_serial.into();
    let new_count = diff.ledger.new_accounts.entries().len() as u32;
    let _new_last = pre_serial
        .checked_add(new_count.saturating_sub(1))
        .ok_or(DaError::InsufficientContext)?;

    let mut last_serial: Option<u32> = None;
    for entry in diff.ledger.account_diffs.entries() {
        let serial: u32 = entry.account_serial.into();
        if serial >= pre_serial {
            return Err(DaError::InsufficientContext);
        }
        if let Some(prev) = last_serial
            && serial <= prev
        {
            return Err(DaError::InsufficientContext);
        }
        last_serial = Some(serial);
    }
    Ok(())
}

fn apply_account_diff<S: IStateAccessor>(
    target: &mut S,
    account_id: AccountId,
    diff: &AccountDiff,
) -> Result<(), DaError> {
    target
        .update_account(account_id, |acct| apply_account_diff_to_account(acct, diff))
        .map_err(|_| DaError::InsufficientContext)?
}

fn apply_account_diff_to_account<T: IAccountStateMut>(
    acct: &mut T,
    diff: &AccountDiff,
) -> Result<(), DaError> {
    if let Some(new_balance) = diff.balance.new_value() {
        apply_balance(acct, *new_balance)?;
    }

    apply_snark_diff(acct, &diff.snark)?;
    Ok(())
}

fn apply_balance<T: IAccountStateMut>(
    acct: &mut T,
    new_balance: BitcoinAmount,
) -> Result<(), DaError> {
    let current = acct.balance();
    if new_balance > current {
        let delta = new_balance
            .checked_sub(current)
            .ok_or(DaError::InsufficientContext)?;
        let coin = Coin::new_unchecked(delta);
        acct.add_balance(coin);
    } else if new_balance < current {
        let delta = current
            .checked_sub(new_balance)
            .ok_or(DaError::InsufficientContext)?;
        acct.take_balance(delta)
            .map_err(|_| DaError::InsufficientContext)?;
    }
    Ok(())
}

fn apply_snark_diff<T: IAccountStateMut>(
    acct: &mut T,
    diff: &SnarkAccountDiff,
) -> Result<(), DaError> {
    if diff.seq_no.diff().is_none()
        && diff.proof_state.new_value().is_none()
        && diff.inbox.new_entries().is_empty()
    {
        return Ok(());
    }

    let snark = acct
        .as_snark_account_mut()
        .map_err(|_| DaError::InsufficientContext)?;

    let mut seq_no = *snark.seqno().inner();
    diff.seq_no.apply(&mut seq_no, &())?;
    let next_seqno = Seqno::new(seq_no);

    let current_proof_state =
        DaProofState::new(snark.inner_state_root(), snark.next_inbox_msg_idx());
    let next_proof_state = diff
        .proof_state
        .new_value()
        .cloned()
        .unwrap_or(current_proof_state);
    snark.set_proof_state_directly(
        next_proof_state.inner().inner_state(),
        next_proof_state.inner().next_inbox_msg_idx(),
        next_seqno,
    );

    for entry in diff.inbox.new_entries() {
        let msg = MessageEntry::new(entry.source, entry.incl_epoch, entry.payload.clone());
        snark
            .insert_inbox_message(msg)
            .map_err(|_| DaError::InsufficientContext)?;
    }

    Ok(())
}
