//! Top-level DA payload types.

use std::{collections::BTreeSet, marker::PhantomData};

use strata_acct_types::{AccountId, BitcoinAmount, MessageEntry};
use strata_codec::{Codec, CodecError, decode_buf_exact};
use strata_da_framework::{DaError as FrameworkDaError, DaWrite, SignedVarInt};
use strata_identifiers::AccountSerial;
use strata_ledger_types::*;
use strata_predicate::PredicateKeyBuf;
use strata_snark_acct_types::Seqno;

use super::{
    AccountDiff, AccountInit, AccountTypeInit, DaProofState, GlobalStateDiff, LedgerDiff,
    SnarkAccountDiff,
};
use crate::DaError;

/// Versioned OL DA payload containing the state diff.
///
/// Wire format is `strata_codec` (not SSZ).
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

/// Decodes [`OLDaPayloadV1`] from raw bytes using exact `strata_codec` decoding.
pub fn decode_ol_da_payload_bytes(bytes: &[u8]) -> Result<OLDaPayloadV1, CodecError> {
    decode_buf_exact(bytes)
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

/// Adapter for applying a state diff to a concrete state accessor.
#[derive(Debug)]
pub struct OLStateDiff<S: IStateAccessorMut> {
    diff: StateDiff,
    _target: PhantomData<S>,
}

impl<S: IStateAccessorMut> OLStateDiff<S> {
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

impl<S: IStateAccessorMut> Default for OLStateDiff<S> {
    fn default() -> Self {
        Self::new(StateDiff::default())
    }
}

impl<S: IStateAccessorMut> DaWrite for OLStateDiff<S> {
    type Target = S;
    type Context = ();
    type Error = DaError;

    fn is_default(&self) -> bool {
        DaWrite::is_default(&self.diff.global) && self.diff.ledger.is_empty()
    }

    fn poll_context(
        &self,
        target: &Self::Target,
        _context: &Self::Context,
    ) -> Result<(), Self::Error> {
        let pre_state_next_serial = target.next_account_serial();
        validate_ledger_entries(pre_state_next_serial, &self.diff)?;
        for entry in self.diff.ledger.new_accounts.entries() {
            new_account_data_from_init(&entry.init)?;
            let exists = target
                .check_account_exists(entry.account_id)
                .map_err(|_| FrameworkDaError::InsufficientContext)?;
            if exists {
                return Err(DaError::InvalidLedgerDiff("new account already exists"));
            }
        }

        for diff in self.diff.ledger.account_diffs.entries() {
            target
                .find_account_id_by_serial(diff.account_serial)
                .map_err(|_| FrameworkDaError::InsufficientContext)?
                .ok_or(FrameworkDaError::InsufficientContext)?;
        }
        Ok(())
    }

    fn apply(
        &self,
        target: &mut Self::Target,
        _context: &Self::Context,
    ) -> Result<(), Self::Error> {
        let mut cur_slot = target.cur_slot();
        self.diff.global.cur_slot.apply(&mut cur_slot, &())?;
        target.set_cur_slot(cur_slot);

        let pre_state_next_serial = target.next_account_serial();
        // NOTE: `validate_ledger_entries` is intentionally not called here;
        // it was already called in `poll_context` which runs before `apply`.
        let mut expected_serial = pre_state_next_serial;
        for entry in self.diff.ledger.new_accounts.entries() {
            let exists = target
                .check_account_exists(entry.account_id)
                .map_err(|_| FrameworkDaError::InsufficientContext)?;
            if exists {
                return Err(DaError::InvalidLedgerDiff("new account already exists"));
            }
            let new_acct = new_account_data_from_init(&entry.init)?;
            let serial = target
                .create_new_account(entry.account_id, new_acct)
                .map_err(|_| DaError::InvalidLedgerDiff("failed to create new account"))?;
            if serial != expected_serial {
                return Err(DaError::InvalidLedgerDiff("new account serial mismatch"));
            }
            expected_serial = expected_serial.incr();
        }

        for entry in self.diff.ledger.account_diffs.entries() {
            let account_id = target
                .find_account_id_by_serial(entry.account_serial)
                .map_err(|_| FrameworkDaError::InsufficientContext)?
                .ok_or(FrameworkDaError::InsufficientContext)?;
            apply_account_diff(target, account_id, &entry.diff)?;
        }
        Ok(())
    }
}

fn new_account_data_from_init(init: &AccountInit) -> Result<NewAccountData, DaError> {
    let type_state = match &init.type_state {
        AccountTypeInit::Empty => NewAccountTypeState::Empty,
        AccountTypeInit::Snark(snark) => {
            let buf = PredicateKeyBuf::try_from(snark.update_vk.as_slice())
                .map_err(|_| DaError::InvalidLedgerDiff("invalid predicate key"))?;
            NewAccountTypeState::Snark {
                update_vk: buf.to_owned(),
                initial_state_root: snark.initial_state_root,
            }
        }
    };
    Ok(NewAccountData::new(init.balance, type_state))
}

fn validate_ledger_entries(
    pre_state_next_serial: AccountSerial,
    diff: &StateDiff,
) -> Result<(), DaError> {
    let mut seen_new_ids = BTreeSet::new();
    for entry in diff.ledger.new_accounts.entries() {
        if !seen_new_ids.insert(entry.account_id) {
            return Err(DaError::InvalidLedgerDiff("duplicate new account id"));
        }
    }

    let pre_serial: u32 = pre_state_next_serial.into();
    let new_count = diff.ledger.new_accounts.entries().len() as u32;
    if new_count > 0 {
        pre_serial
            .checked_add(new_count - 1)
            .ok_or(DaError::InvalidLedgerDiff(
                "new account serial range overflows",
            ))?;
    }

    let mut last_serial: Option<u32> = None;
    for entry in diff.ledger.account_diffs.entries() {
        let serial: u32 = entry.account_serial.into();
        if serial >= pre_serial {
            return Err(DaError::InvalidLedgerDiff(
                "account diff serial out of range",
            ));
        }
        if let Some(prev) = last_serial
            && serial <= prev
        {
            return Err(DaError::InvalidLedgerDiff(
                "account diff serials not strictly increasing",
            ));
        }
        last_serial = Some(serial);
    }
    Ok(())
}

fn apply_account_diff<S: IStateAccessorMut>(
    target: &mut S,
    account_id: AccountId,
    diff: &AccountDiff,
) -> Result<(), DaError> {
    target
        .update_account(account_id, |acct| apply_account_diff_to_account(acct, diff))
        .map_err(|_| DaError::InvalidStateDiff("failed to update account diff"))?
}

fn apply_account_diff_to_account<T: IAccountStateMut>(
    acct: &mut T,
    diff: &AccountDiff,
) -> Result<(), DaError> {
    if let Some(incr) = diff.balance.diff() {
        apply_balance_delta(acct, incr)?;
    }

    if !DaWrite::is_default(&diff.snark) {
        apply_snark_diff(acct, &diff.snark)?;
    }
    Ok(())
}

fn apply_balance_delta<T: IAccountStateMut>(
    acct: &mut T,
    incr: &SignedVarInt,
) -> Result<(), DaError> {
    if incr.is_positive() {
        let delta = BitcoinAmount::from_sat(incr.magnitude());
        let coin = Coin::new_unchecked(delta);
        acct.add_balance(coin);
    } else {
        let delta = BitcoinAmount::from_sat(incr.magnitude());
        acct.take_balance(delta)
            .map_err(|_| DaError::InvalidStateDiff("insufficient balance for diff"))?;
    }
    Ok(())
}

fn apply_snark_diff<T: IAccountStateMut>(
    acct: &mut T,
    diff: &SnarkAccountDiff,
) -> Result<(), DaError> {
    let snark = acct
        .as_snark_account_mut()
        .map_err(|_| DaError::InvalidStateDiff("snark diff applied to non-snark account"))?;

    let mut seq_no = *snark.seqno().inner();
    diff.seq_no.apply(&mut seq_no, &())?;
    let next_seqno = Seqno::new(seq_no);

    let mut next_proof_state =
        DaProofState::new(snark.inner_state_root(), snark.next_inbox_msg_idx());
    diff.proof_state.apply(&mut next_proof_state, &())?;
    snark.set_proof_state_directly(
        next_proof_state.inner().inner_state(),
        next_proof_state.inner().next_inbox_msg_idx(),
        next_seqno,
    );

    for entry in diff.inbox.new_entries() {
        let msg = MessageEntry::new(entry.source, entry.incl_epoch, entry.payload.clone());
        snark
            .insert_inbox_message(msg)
            .map_err(|_| DaError::InvalidStateDiff("failed to insert inbox message"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount, Hash};
    use strata_codec::encode_to_vec;
    use strata_da_framework::{DaCounter, DaLinacc, DaWrite, SignedVarInt, counter_schemes};
    use strata_identifiers::AccountSerial;
    use strata_ledger_types::{IStateAccessor, IStateAccessorMut, NewAccountData};
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_stf::test_utils::create_test_genesis_state;
    use strata_predicate::PredicateKey;

    use super::*;
    use crate::{AccountDiffEntry, DaProofStateDiff, NewAccountEntry, U16LenList};

    fn test_account_id(seed: u8) -> AccountId {
        AccountId::from([seed; 32])
    }

    #[test]
    fn test_payload_encodes_state_diff_only() {
        let diff_bytes = encode_to_vec(&StateDiff::default()).expect("encode diff");
        let payload = OLDaPayloadV1::new(StateDiff::default());
        let payload_bytes = encode_to_vec(&payload).expect("encode payload");

        assert_eq!(payload_bytes, diff_bytes);
    }

    #[test]
    fn test_decode_ol_da_payload_bytes_roundtrip() {
        let payload = OLDaPayloadV1::new(StateDiff::default());
        let encoded = encode_to_vec(&payload).expect("encode payload");

        let decoded = decode_ol_da_payload_bytes(&encoded).expect("decode payload");
        let reencoded = encode_to_vec(&decoded).expect("re-encode payload");

        assert_eq!(encoded, reencoded);
    }

    #[test]
    fn test_decode_ol_da_payload_bytes_rejects_trailing_bytes() {
        let payload = OLDaPayloadV1::new(StateDiff::default());
        let mut encoded = encode_to_vec(&payload).expect("encode payload");
        encoded.push(0u8);

        let decoded = decode_ol_da_payload_bytes(&encoded);
        assert!(decoded.is_err());
    }

    #[test]
    fn test_validate_ledger_entries_rejects_duplicate_new_ids() {
        let account_id = test_account_id(1);
        let init = AccountInit::new(BitcoinAmount::from_sat(1), AccountTypeInit::Empty);
        let diff = StateDiff::new(
            GlobalStateDiff::default(),
            LedgerDiff::new(
                U16LenList::new(vec![
                    NewAccountEntry::new(account_id, init.clone()),
                    NewAccountEntry::new(account_id, init),
                ]),
                U16LenList::new(Vec::new()),
            ),
        );

        let result = validate_ledger_entries(AccountSerial::from(1u32), &diff);
        assert!(matches!(
            result,
            Err(DaError::InvalidLedgerDiff("duplicate new account id"))
        ));
    }

    #[test]
    fn test_ol_state_diff_poll_context_rejects_existing_new_account() {
        let mut state = create_test_genesis_state();
        let account_id = test_account_id(2);
        let new_acct = NewAccountData::new(BitcoinAmount::from_sat(10), NewAccountTypeState::Empty);
        state
            .create_new_account(account_id, new_acct)
            .expect("create account");

        let init = AccountInit::new(BitcoinAmount::from_sat(1), AccountTypeInit::Empty);
        let diff = StateDiff::new(
            GlobalStateDiff::default(),
            LedgerDiff::new(
                U16LenList::new(vec![NewAccountEntry::new(account_id, init)]),
                U16LenList::new(Vec::new()),
            ),
        );

        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        let result = DaWrite::poll_context(&ol_diff, &state, &());

        assert!(matches!(
            result,
            Err(DaError::InvalidLedgerDiff("new account already exists"))
        ));
    }

    #[test]
    fn test_ol_state_diff_apply_updates_balance() {
        let mut state = create_test_genesis_state();
        let account_id = test_account_id(3);
        let new_acct =
            NewAccountData::new(BitcoinAmount::from_sat(1_000), NewAccountTypeState::Empty);
        let serial = state
            .create_new_account(account_id, new_acct)
            .expect("create account");

        // Balance goes from 1_000 to 2_000, so the delta is +1_000
        let account_diff = AccountDiff::new(
            DaCounter::new_changed(SignedVarInt::positive(1_000)),
            SnarkAccountDiff::default(),
        );
        let diff = StateDiff::new(
            GlobalStateDiff::default(),
            LedgerDiff::new(
                U16LenList::new(Vec::new()),
                U16LenList::new(vec![AccountDiffEntry::new(serial, account_diff)]),
            ),
        );

        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        DaWrite::apply(&ol_diff, &mut state, &()).expect("apply diff");

        let account = state
            .get_account_state(account_id)
            .expect("read account")
            .expect("account exists");
        assert_eq!(account.balance(), BitcoinAmount::from_sat(2_000));
    }

    #[test]
    fn test_ol_state_diff_apply_snark_seqno() {
        let mut state = create_test_genesis_state();
        let account_id = test_account_id(4);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(500),
            NewAccountTypeState::Snark {
                update_vk: PredicateKey::always_accept(),
                initial_state_root: Hash::from([0x11u8; 32]),
            },
        );
        let serial = state
            .create_new_account(account_id, new_acct)
            .expect("create snark account");

        let snark_diff = SnarkAccountDiff::new(
            DaCounter::<counter_schemes::CtrU64ByU16>::new_changed(1u16),
            DaProofStateDiff::default(),
            DaLinacc::new(),
        );
        let account_diff = AccountDiff::new(DaCounter::new_unchanged(), snark_diff);
        let diff = StateDiff::new(
            GlobalStateDiff::default(),
            LedgerDiff::new(
                U16LenList::new(Vec::new()),
                U16LenList::new(vec![AccountDiffEntry::new(serial, account_diff)]),
            ),
        );

        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        DaWrite::apply(&ol_diff, &mut state, &()).expect("apply snark diff");

        let account = state
            .get_account_state(account_id)
            .expect("read account")
            .expect("account exists");
        let snark = account.as_snark_account().expect("snark account");
        assert_eq!(*snark.seqno().inner(), 1);
    }
}
