//! Top-level DA payload types.

use std::marker::PhantomData;

use ssz::{Decode, Encode};
use strata_acct_types::{AccountId, BitcoinAmount};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{DaError, DaWrite};
use strata_ledger_types::{
    Coin, IAccountStateMut, ISnarkAccountState, ISnarkAccountStateMut, IStateAccessor,
    NewAccountData,
};
use strata_ol_chain_types_new::OLLog;
use strata_snark_acct_types::{MessageEntry, Seqno};

use super::{
    AccountDiff, AccountInit, DaProofState, GlobalStateDiff, LedgerDiff, SnarkAccountDiff,
};

/// Versioned OL DA payload containing the state diff and output logs.
#[derive(Debug, Codec)]
pub struct OLDaPayloadV1 {
    /// State diff for the epoch.
    pub state_diff: StateDiff,

    /// Ordered output logs emitted during the epoch.
    pub output_logs: OutputLogs,
}

impl OLDaPayloadV1 {
    /// Creates a new [`OLDaPayloadV1`] from a state diff.
    pub fn new(state_diff: StateDiff, output_logs: OutputLogs) -> Self {
        Self {
            state_diff,
            output_logs,
        }
    }
}

/// Ordered output logs emitted during the epoch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutputLogs {
    logs: Vec<OLLog>,
}

impl OutputLogs {
    pub fn new(logs: Vec<OLLog>) -> Self {
        Self { logs }
    }

    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    pub fn into_logs(self) -> Vec<OLLog> {
        self.logs
    }
}

impl Codec for OutputLogs {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let len = u16::try_from(self.logs.len()).map_err(|_| CodecError::OverflowContainer)?;
        len.encode(enc)?;
        for log in &self.logs {
            let bytes = log.as_ssz_bytes();
            let log_len = u16::try_from(bytes.len()).map_err(|_| CodecError::OverflowContainer)?;
            log_len.encode(enc)?;
            enc.write_buf(&bytes)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u16::decode(dec)? as usize;
        let mut logs = Vec::with_capacity(len);
        for _ in 0..len {
            let log_len = u16::decode(dec)? as usize;
            let mut buf = vec![0u8; log_len];
            dec.read_buf(&mut buf)?;
            let log =
                OLLog::from_ssz_bytes(&buf).map_err(|_| CodecError::InvalidVariant("ol_log"))?;
            logs.push(log);
        }
        Ok(Self { logs })
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
