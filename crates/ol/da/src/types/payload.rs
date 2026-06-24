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
/// Wire format is the `strata_codec` encoding of [`StateDiff`] (not SSZ).
///
/// # Compatibility window
///
/// V1 is the only format currently produced or consumed; there is no V2 and no in-band version
/// byte. The byte layout is frozen by the golden fixture in this module's tests: any change to the
/// encoding of [`StateDiff`] or its nested types breaks that test by design. Such a change is a
/// wire-format break that requires a new payload version, not an edit to V1; on an intentional
/// break, introduce the new version and regenerate the fixture rather than mutating V1 in place.
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

/// Epoch OL state diff (global + ledger).
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

        if let Some(d) = self.diff.global.limbo_funds_sats.diff() {
            let amt = BitcoinAmount::from_sat(d.magnitude());
            if d.is_positive() {
                let coin = Coin::new_unchecked(amt);
                target
                    .add_limbo_funds_coin(coin)
                    .map_err(|_| DaError::InvalidStateDiff("failed to apply limbo funds diff"))?;
            } else {
                let coin = target
                    .take_limbo_funds_coin(amt)
                    .map_err(|_| DaError::InvalidStateDiff("insufficient limbo funds for diff"))?;
                coin.safely_consume_unchecked();
            }
        }

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
        let coin = acct
            .take_balance(delta)
            .map_err(|_| DaError::InvalidStateDiff("insufficient balance for diff"))?;
        coin.safely_consume_unchecked();
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
    snark.set_proof_state(
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
    use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_da_framework::{
        DaCounter, DaLinacc, DaRegister, DaWrite, SignedVarInt, UnsignedVarInt, counter_schemes,
    };
    use strata_identifiers::AccountSerial;
    use strata_ledger_types::{IStateAccessor, IStateAccessorMut, NewAccountData};
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_stf::test_utils::make_genesis_state;
    use strata_predicate::PredicateKey;

    use super::{
        super::{MAX_MSG_PAYLOAD_BYTES, MAX_VK_BYTES},
        *,
    };
    use crate::{
        AccountDiffEntry, DaMessageEntry, DaProofStateDiff, DaScheme, NewAccountEntry,
        OLDaSchemeV1, SnarkAccountInit, U16LenList,
    };

    fn test_account_id(seed: u8) -> AccountId {
        AccountId::from([seed; 32])
    }

    /// Builders for valid test OL DA diff trees.
    mod build {
        use super::*;

        /// Empty new-account init with the given balance.
        pub(super) fn empty_init(balance_sats: u64) -> AccountInit {
            AccountInit::new(
                BitcoinAmount::from_sat(balance_sats),
                AccountTypeInit::Empty,
            )
        }

        /// Snark new-account init with the given balance, state root, and VK bytes.
        pub(super) fn snark_init(balance_sats: u64, root: Hash, vk: Vec<u8>) -> AccountInit {
            AccountInit::new(
                BitcoinAmount::from_sat(balance_sats),
                AccountTypeInit::Snark(SnarkAccountInit::new(root, vk)),
            )
        }

        /// Balance-only account diff with a signed sats delta.
        pub(super) fn balance_diff(delta: SignedVarInt) -> AccountDiff {
            AccountDiff::new(DaCounter::new_changed(delta), SnarkAccountDiff::default())
        }

        /// Snark diff with seqno, proof-state, and inbox changes.
        pub(super) fn snark_diff(
            seqno_incr: u16,
            new_root: Option<Hash>,
            next_idx_incr: u64,
            inbox_msgs: Vec<DaMessageEntry>,
        ) -> SnarkAccountDiff {
            let inner_state = match new_root {
                Some(r) => DaRegister::new_set(r),
                None => DaRegister::new_unset(),
            };
            let next_idx = if next_idx_incr == 0 {
                DaCounter::new_unchanged()
            } else {
                DaCounter::new_changed(UnsignedVarInt::new(next_idx_incr))
            };
            let proof_state = DaProofStateDiff::new(inner_state, next_idx);
            let mut inbox = DaLinacc::new();
            for m in inbox_msgs {
                assert!(inbox.append_entry(m), "inbox write should accept entry");
            }
            let seq_no = if seqno_incr == 0 {
                DaCounter::<counter_schemes::CtrU64ByU16>::new_unchanged()
            } else {
                DaCounter::new_changed(seqno_incr)
            };
            SnarkAccountDiff::new(seq_no, proof_state, inbox)
        }

        /// Inbox message entry with a repeated-byte payload.
        pub(super) fn inbox_msg(
            source: AccountId,
            incl_epoch: u32,
            len: usize,
            byte: u8,
        ) -> DaMessageEntry {
            let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(0), vec![byte; len])
                .expect("message payload bytes must fit SSZ max length");
            DaMessageEntry::new(source, incl_epoch, payload)
        }

        /// Ledger diff from new accounts and account diffs.
        pub(super) fn ledger(
            new_accounts: Vec<NewAccountEntry>,
            account_diffs: Vec<AccountDiffEntry>,
        ) -> LedgerDiff {
            LedgerDiff::new(
                U16LenList::new(new_accounts),
                U16LenList::new(account_diffs),
            )
        }

        /// Global diff with optional slot and limbo deltas.
        pub(super) fn global(slot_incr: u16, limbo_delta: Option<SignedVarInt>) -> GlobalStateDiff {
            let cur_slot = if slot_incr == 0 {
                DaCounter::new_unchanged()
            } else {
                DaCounter::new_changed(slot_incr)
            };
            let limbo = match limbo_delta {
                Some(d) => DaCounter::new_changed(d),
                None => DaCounter::new_unchanged(),
            };
            GlobalStateDiff::new(cur_slot, limbo)
        }
    }

    /// Shared non-trivial payload fixture for round-trip and golden tests.
    fn populated_state_diff() -> StateDiff {
        let snark_acct = build::snark_init(
            500,
            Hash::from([0x11u8; 32]),
            PredicateKey::always_accept().as_buf_ref().to_bytes(),
        );
        let new_accounts = vec![
            NewAccountEntry::new(test_account_id(0xA1), build::empty_init(1_000)),
            NewAccountEntry::new(test_account_id(0xA2), snark_acct),
        ];

        let inbox = vec![
            build::inbox_msg(test_account_id(0xB1), 7, 4, 0xEE),
            build::inbox_msg(test_account_id(0xB2), 8, 16, 0xCD),
        ];
        let account_diffs = vec![
            AccountDiffEntry::new(
                AccountSerial::from(0u32),
                build::balance_diff(SignedVarInt::positive(250)),
            ),
            AccountDiffEntry::new(
                AccountSerial::from(1u32),
                AccountDiff::new(
                    DaCounter::new_unchanged(),
                    build::snark_diff(3, Some(Hash::from([0x22u8; 32])), 2, inbox),
                ),
            ),
        ];

        StateDiff::new(
            build::global(5, Some(SignedVarInt::positive(900))),
            build::ledger(new_accounts, account_diffs),
        )
    }

    #[test]
    fn test_populated_payload_round_trip() {
        let payload = OLDaPayloadV1::new(populated_state_diff());
        let encoded = encode_to_vec(&payload).expect("encode populated payload");

        let decoded = decode_ol_da_payload_bytes(&encoded).expect("decode populated payload");
        let reencoded = encode_to_vec(&decoded).expect("re-encode populated payload");

        assert_eq!(encoded, reencoded);
    }

    #[test]
    fn test_account_init_round_trip_empty_and_snark() {
        for init in [
            build::empty_init(42),
            build::snark_init(7, Hash::from([0x33u8; 32]), vec![0xAB; 64]),
        ] {
            let encoded = encode_to_vec(&init).expect("encode account init");
            let decoded: AccountInit = decode_buf_exact(&encoded).expect("decode account init");
            assert_eq!(decoded, init);
        }
    }

    #[test]
    fn test_snark_account_diff_round_trip() {
        let inbox = vec![build::inbox_msg(test_account_id(9), 1, 8, 0x55)];
        let diff = build::snark_diff(2, Some(Hash::from([0x44u8; 32])), 1, inbox);
        let encoded = encode_to_vec(&diff).expect("encode snark diff");
        let decoded: SnarkAccountDiff = decode_buf_exact(&encoded).expect("decode snark diff");
        let reencoded = encode_to_vec(&decoded).expect("re-encode snark diff");
        assert_eq!(encoded, reencoded);
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
        let mut state = make_genesis_state();
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
        let mut state = make_genesis_state();
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
    fn test_ol_state_diff_apply_decreases_balance() {
        let mut state = make_genesis_state();
        let account_id = test_account_id(30);
        let new_acct =
            NewAccountData::new(BitcoinAmount::from_sat(2_000), NewAccountTypeState::Empty);
        let serial = state
            .create_new_account(account_id, new_acct)
            .expect("create account");

        let account_diff = AccountDiff::new(
            DaCounter::new_changed(SignedVarInt::negative(750)),
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
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1_250));
    }

    #[test]
    fn test_ol_state_diff_apply_rejects_insufficient_balance() {
        let mut state = make_genesis_state();
        let account_id = test_account_id(31);
        let new_acct =
            NewAccountData::new(BitcoinAmount::from_sat(500), NewAccountTypeState::Empty);
        let serial = state
            .create_new_account(account_id, new_acct)
            .expect("create account");

        let account_diff = build::balance_diff(SignedVarInt::negative(501));
        let diff = StateDiff::new(
            GlobalStateDiff::default(),
            build::ledger(
                Vec::new(),
                vec![AccountDiffEntry::new(serial, account_diff)],
            ),
        );

        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        let result = DaWrite::apply(&ol_diff, &mut state, &());

        assert!(matches!(
            result,
            Err(DaError::InvalidStateDiff("insufficient balance for diff"))
        ));
        let account = state
            .get_account_state(account_id)
            .expect("read account")
            .expect("account exists");
        assert_eq!(account.balance(), BitcoinAmount::from_sat(500));
    }

    #[test]
    fn test_ol_state_diff_apply_updates_limbo_funds() {
        let mut state = make_genesis_state();
        assert_eq!(state.limbo_funds(), BitcoinAmount::from_sat(0));

        let global_diff = GlobalStateDiff::new(
            DaCounter::new_unchanged(),
            DaCounter::new_changed(SignedVarInt::positive(1_500)),
        );
        let diff = StateDiff::new(global_diff, LedgerDiff::default());

        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        DaWrite::apply(&ol_diff, &mut state, &()).expect("apply limbo add diff");

        assert_eq!(state.limbo_funds(), BitcoinAmount::from_sat(1_500));

        let global_diff = GlobalStateDiff::new(
            DaCounter::new_unchanged(),
            DaCounter::new_changed(SignedVarInt::negative(400)),
        );
        let diff = StateDiff::new(global_diff, LedgerDiff::default());

        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        DaWrite::apply(&ol_diff, &mut state, &()).expect("apply limbo take diff");

        assert_eq!(state.limbo_funds(), BitcoinAmount::from_sat(1_100));
    }

    #[test]
    fn test_ol_state_diff_apply_rejects_insufficient_limbo_funds() {
        let mut state = make_genesis_state();
        assert_eq!(state.limbo_funds(), BitcoinAmount::from_sat(0));

        let global_diff = build::global(0, Some(SignedVarInt::negative(1)));
        let diff = StateDiff::new(global_diff, LedgerDiff::default());

        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        let result = DaWrite::apply(&ol_diff, &mut state, &());

        assert!(matches!(
            result,
            Err(DaError::InvalidStateDiff(
                "insufficient limbo funds for diff"
            ))
        ));
        assert_eq!(state.limbo_funds(), BitcoinAmount::from_sat(0));
    }

    #[test]
    fn test_ol_state_diff_apply_snark_seqno() {
        let mut state = make_genesis_state();
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

    #[derive(Clone, Copy)]
    struct AcctRef {
        id: AccountId,
        serial: AccountSerial,
    }

    struct PreStateAccounts {
        state: MemoryStateBaseLayer,
        empty: AcctRef,
        snark: AcctRef,
    }

    /// Returns genesis plus one empty account and one snark account, preserving created serials.
    fn pre_state_with_accounts() -> PreStateAccounts {
        let mut state = make_genesis_state();
        let empty_id = test_account_id(0x10);
        let snark_id = test_account_id(0x11);

        let empty_serial = state
            .create_new_account(
                empty_id,
                NewAccountData::new(BitcoinAmount::from_sat(1_000), NewAccountTypeState::Empty),
            )
            .expect("create empty account");
        let snark_serial = state
            .create_new_account(
                snark_id,
                NewAccountData::new(
                    BitcoinAmount::from_sat(500),
                    NewAccountTypeState::Snark {
                        update_vk: PredicateKey::always_accept(),
                        initial_state_root: Hash::from([0x11u8; 32]),
                    },
                ),
            )
            .expect("create snark account");
        PreStateAccounts {
            state,
            empty: AcctRef {
                id: empty_id,
                serial: empty_serial,
            },
            snark: AcctRef {
                id: snark_id,
                serial: snark_serial,
            },
        }
    }

    /// Compares DA apply with equivalent direct state mutations.
    #[test]
    fn test_apply_equivalence_global_and_accounts() {
        let pre_accounts = pre_state_with_accounts();
        let new_id = test_account_id(0x20);
        let inbox_root = Hash::from([0x22u8; 32]);
        let inbox_msg = build::inbox_msg(test_account_id(0x30), 9, 12, 0x7A);

        let mut applied = pre_accounts.state.clone();
        let diff = StateDiff::new(
            build::global(4, Some(SignedVarInt::positive(800))),
            build::ledger(
                vec![NewAccountEntry::new(new_id, build::empty_init(2_000))],
                vec![
                    AccountDiffEntry::new(
                        pre_accounts.empty.serial,
                        build::balance_diff(SignedVarInt::positive(250)),
                    ),
                    AccountDiffEntry::new(
                        pre_accounts.snark.serial,
                        AccountDiff::new(
                            DaCounter::new_unchanged(),
                            build::snark_diff(2, Some(inbox_root), 1, vec![inbox_msg.clone()]),
                        ),
                    ),
                ],
            ),
        );
        OLDaSchemeV1::apply_to_state(OLDaPayloadV1::new(diff), &mut applied)
            .expect("apply diff via scheme");

        let mut expected = pre_accounts.state.clone();
        expected.set_cur_slot(expected.cur_slot() + 4);
        expected
            .add_limbo_funds_coin(Coin::new_unchecked(BitcoinAmount::from_sat(800)))
            .expect("add limbo");
        expected
            .create_new_account(
                new_id,
                NewAccountData::new(BitcoinAmount::from_sat(2_000), NewAccountTypeState::Empty),
            )
            .expect("create new account");
        expected
            .update_account(pre_accounts.empty.id, |acct| {
                acct.add_balance(Coin::new_unchecked(BitcoinAmount::from_sat(250)));
                Ok::<(), DaError>(())
            })
            .expect("update empty")
            .expect("balance ok");
        expected
            .update_account(pre_accounts.snark.id, |acct| {
                let snark = acct.as_snark_account_mut().expect("snark");
                let next_seqno = Seqno::new(*snark.seqno().inner() + 2);
                snark.set_proof_state(inbox_root, snark.next_inbox_msg_idx() + 1, next_seqno);
                snark
                    .insert_inbox_message(MessageEntry::new(
                        inbox_msg.source,
                        inbox_msg.incl_epoch,
                        inbox_msg.payload.clone(),
                    ))
                    .expect("insert inbox");
                Ok::<(), DaError>(())
            })
            .expect("update snark")
            .expect("snark ok");

        assert_eq!(
            applied.compute_state_root().expect("applied root"),
            expected.compute_state_root().expect("expected root"),
        );

        assert_eq!(applied.cur_slot(), expected.cur_slot());
        assert_eq!(applied.limbo_funds(), expected.limbo_funds());
        for id in [pre_accounts.empty.id, pre_accounts.snark.id, new_id] {
            let a = applied
                .get_account_state(id)
                .unwrap()
                .expect("applied acct");
            let e = expected
                .get_account_state(id)
                .unwrap()
                .expect("expected acct");
            assert_eq!(a.balance(), e.balance(), "balance mismatch for {id:?}");
        }
        let a_snark = applied
            .get_account_state(pre_accounts.snark.id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .expect("snark");
        assert_eq!(*a_snark.seqno().inner(), 2);
        assert_eq!(a_snark.inner_state_root(), inbox_root);
        assert_eq!(a_snark.next_inbox_msg_idx(), 1);
    }

    #[test]
    fn test_apply_empty_diff_is_noop() {
        let pre_accounts = pre_state_with_accounts();
        let before_root = pre_accounts
            .state
            .compute_state_root()
            .expect("root before");

        let mut state = pre_accounts.state.clone();
        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(StateDiff::default());
        assert!(DaWrite::is_default(&ol_diff));
        OLDaSchemeV1::apply_to_state(OLDaPayloadV1::new(StateDiff::default()), &mut state)
            .expect("apply empty diff");

        assert_eq!(state.compute_state_root().expect("root after"), before_root);
    }

    #[test]
    fn test_validate_rejects_account_diff_serial_out_of_range() {
        let diff = account_diffs_at(&[5]);
        let result = validate_ledger_entries(AccountSerial::from(5u32), &diff);
        assert!(matches!(
            result,
            Err(DaError::InvalidLedgerDiff(
                "account diff serial out of range"
            ))
        ));
    }

    /// Builds account diffs with exactly the given serials, in order.
    fn account_diffs_at(serials: &[u32]) -> StateDiff {
        StateDiff::new(
            GlobalStateDiff::default(),
            build::ledger(
                Vec::new(),
                serials
                    .iter()
                    .map(|s| {
                        AccountDiffEntry::new(
                            AccountSerial::from(*s),
                            build::balance_diff(SignedVarInt::positive(1)),
                        )
                    })
                    .collect(),
            ),
        )
    }

    #[test]
    fn test_validate_rejects_duplicate_account_diff_serial() {
        let diff = account_diffs_at(&[1, 1]);
        let result = validate_ledger_entries(AccountSerial::from(5u32), &diff);
        assert!(matches!(
            result,
            Err(DaError::InvalidLedgerDiff(
                "account diff serials not strictly increasing"
            ))
        ));
    }

    #[test]
    fn test_validate_rejects_decreasing_account_diff_serial() {
        let diff = account_diffs_at(&[3, 2]);
        let result = validate_ledger_entries(AccountSerial::from(5u32), &diff);
        assert!(matches!(
            result,
            Err(DaError::InvalidLedgerDiff(
                "account diff serials not strictly increasing"
            ))
        ));
    }

    /// Account-diff serials need only be strictly increasing and in range.
    #[test]
    fn test_validate_accepts_gapped_account_diff_serials() {
        let diff = account_diffs_at(&[1, 3]);
        assert!(validate_ledger_entries(AccountSerial::from(5u32), &diff).is_ok());
    }

    #[test]
    fn test_apply_rejects_snark_diff_on_non_snark_account() {
        let pre_accounts = pre_state_with_accounts();
        let mut state = pre_accounts.state.clone();

        let diff = StateDiff::new(
            GlobalStateDiff::default(),
            build::ledger(
                Vec::new(),
                vec![AccountDiffEntry::new(
                    pre_accounts.empty.serial,
                    AccountDiff::new(
                        DaCounter::new_unchanged(),
                        build::snark_diff(1, None, 0, Vec::new()),
                    ),
                )],
            ),
        );
        let result = OLDaSchemeV1::apply_to_state(OLDaPayloadV1::new(diff), &mut state);
        assert!(matches!(
            result,
            Err(DaError::InvalidStateDiff(
                "snark diff applied to non-snark account"
            ))
        ));
    }

    #[test]
    fn test_poll_context_rejects_malformed_vk() {
        let pre_accounts = pre_state_with_accounts();
        let bad_vk = vec![0xFFu8; 3];
        let init = AccountInit::new(
            BitcoinAmount::from_sat(1),
            AccountTypeInit::Snark(SnarkAccountInit::new(Hash::from([0u8; 32]), bad_vk)),
        );
        let diff = StateDiff::new(
            GlobalStateDiff::default(),
            build::ledger(
                vec![NewAccountEntry::new(test_account_id(0x40), init)],
                Vec::new(),
            ),
        );
        let ol_diff = OLStateDiff::<MemoryStateBaseLayer>::new(diff);
        let result = DaWrite::poll_context(&ol_diff, &pre_accounts.state, &());
        assert!(matches!(
            result,
            Err(DaError::InvalidLedgerDiff("invalid predicate key"))
        ));
    }

    #[test]
    fn test_snark_init_vk_round_trips_at_max_len() {
        // MAX_VK_BYTES is the largest value representable by the u16 length prefix.
        let vk = vec![0xABu8; MAX_VK_BYTES];
        let init = SnarkAccountInit::new(Hash::from([1u8; 32]), vk);
        let encoded = encode_to_vec(&init).expect("encode max-len vk");
        let decoded: SnarkAccountInit = decode_buf_exact(&encoded).expect("decode max-len vk");
        assert_eq!(decoded, init);
    }

    #[test]
    fn test_da_message_entry_round_trips_at_max_payload() {
        let payload = MsgPayload::from_bytes(
            BitcoinAmount::from_sat(0),
            vec![0x5Au8; MAX_MSG_PAYLOAD_BYTES],
        )
        .expect("payload at boundary fits SSZ max");
        let entry = DaMessageEntry::new(test_account_id(7), 3, payload);
        let encoded = encode_to_vec(&entry).expect("encode boundary entry");
        let decoded: DaMessageEntry = decode_buf_exact(&encoded).expect("decode boundary entry");
        assert_eq!(decoded, entry);
    }

    /// Frozen wire-format fixture for [`OLDaPayloadV1`].
    ///
    /// The hex was derived from the encoder and acts as a drift detector, not a hand-verified spec
    /// oracle.
    const GOLDEN_PAYLOAD_V1_HEX: &str = "030005840e0002a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a100000000000003e800a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a200000000000001f401111111111111111111111111111111111111111111111111111111111111111100010100020000000001ba030000000102070003032222222222222222222222222222222222222222222222222222222222222222020002b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b100000007000000000000000004eeeeeeeeb2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b200000008000000000000000010cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
            .collect()
    }

    #[test]
    fn test_golden_payload_v1_wire_format_is_stable() {
        let golden = hex_to_bytes(GOLDEN_PAYLOAD_V1_HEX);

        let encoded = encode_to_vec(&OLDaPayloadV1::new(populated_state_diff()))
            .expect("encode populated payload");
        assert_eq!(
            encoded, golden,
            "wire format drifted from the golden fixture; if intentional, regenerate the constant \
             and bump the compatibility note on OLDaPayloadV1"
        );

        let decoded = decode_ol_da_payload_bytes(&golden).expect("decode golden payload");
        let reencoded = encode_to_vec(&decoded).expect("re-encode decoded golden");
        assert_eq!(reencoded, golden);
    }
}
