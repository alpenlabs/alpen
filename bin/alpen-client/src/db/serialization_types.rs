use borsh::{BorshDeserialize, BorshSerialize};
use strata_acct_types::{BitcoinAmount, Hash, SubjectId};
use strata_ee_acct_types::{EeAccountState, PendingFinclEntry, PendingInputEntry};
use strata_ee_chain_types::SubjectDepositData;
use strata_identifiers::{Buf32, OLBlockId};

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBOLBlockId(Buf32);

impl From<OLBlockId> for DBOLBlockId {
    fn from(value: OLBlockId) -> Self {
        Self(value.into())
    }
}

impl From<DBOLBlockId> for OLBlockId {
    fn from(value: DBOLBlockId) -> Self {
        value.0.into()
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBAccountStateAtSlot {
    slot: u64,
    account_state: DBEeAccountState,
}

impl DBAccountStateAtSlot {
    pub(crate) fn from_parts(slot: u64, account_state: DBEeAccountState) -> Self {
        Self {
            slot,
            account_state,
        }
    }

    pub(crate) fn into_parts(self) -> (u64, DBEeAccountState) {
        (self.slot, self.account_state)
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBEeAccountState {
    last_exec_blkid: Hash,
    tracked_balance: DBBitcoinAmount,
    pending_inputs: Vec<DBPendingInputEntry>,
    pending_fincls: Vec<DBPendingFinclEntry>,
}

impl From<EeAccountState> for DBEeAccountState {
    fn from(value: EeAccountState) -> Self {
        let (last_exec_blkid, tracked_balance, pending_inputs, pending_fincls) = value.into_parts();
        Self {
            last_exec_blkid,
            tracked_balance: tracked_balance.into(),
            pending_inputs: pending_inputs.into_iter().map(Into::into).collect(),
            pending_fincls: pending_fincls.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<DBEeAccountState> for EeAccountState {
    fn from(value: DBEeAccountState) -> Self {
        Self::new(
            value.last_exec_blkid,
            value.tracked_balance.into(),
            value.pending_inputs.into_iter().map(Into::into).collect(),
            value.pending_fincls.into_iter().map(Into::into).collect(),
        )
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
struct DBBitcoinAmount(u64);

impl From<DBBitcoinAmount> for BitcoinAmount {
    fn from(value: DBBitcoinAmount) -> Self {
        Self::from_sat(value.0)
    }
}

impl From<BitcoinAmount> for DBBitcoinAmount {
    fn from(value: BitcoinAmount) -> Self {
        Self(value.to_sat())
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
struct DBPendingFinclEntry {
    epoch: u32,
    raw_tx_hash: Hash,
}

impl From<PendingFinclEntry> for DBPendingFinclEntry {
    fn from(value: PendingFinclEntry) -> Self {
        let (epoch, raw_tx_hash) = value.into_parts();
        Self { epoch, raw_tx_hash }
    }
}

impl From<DBPendingFinclEntry> for PendingFinclEntry {
    fn from(value: DBPendingFinclEntry) -> Self {
        let DBPendingFinclEntry { epoch, raw_tx_hash } = value;
        Self::new(epoch, raw_tx_hash)
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
enum DBPendingInputEntry {
    Deposit(DBSubjectDepositData),
}

impl From<DBPendingInputEntry> for PendingInputEntry {
    fn from(value: DBPendingInputEntry) -> Self {
        match value {
            DBPendingInputEntry::Deposit(value) => Self::Deposit(value.into()),
        }
    }
}

impl From<PendingInputEntry> for DBPendingInputEntry {
    fn from(value: PendingInputEntry) -> Self {
        match value {
            PendingInputEntry::Deposit(value) => Self::Deposit(value.into()),
        }
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
struct DBSubjectDepositData {
    dest: DBSubjectId,
    value: DBBitcoinAmount,
}

impl From<DBSubjectDepositData> for SubjectDepositData {
    fn from(value: DBSubjectDepositData) -> Self {
        Self::new(value.dest.into(), value.value.into())
    }
}

impl From<SubjectDepositData> for DBSubjectDepositData {
    fn from(value: SubjectDepositData) -> Self {
        Self {
            dest: value.dest().into(),
            value: value.value().into(),
        }
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
struct DBSubjectId([u8; 32]);

impl From<DBSubjectId> for SubjectId {
    fn from(value: DBSubjectId) -> Self {
        Self::new(value.0)
    }
}

impl From<SubjectId> for DBSubjectId {
    fn from(value: SubjectId) -> Self {
        Self(value.into())
    }
}
