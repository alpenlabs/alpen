use serde::{Deserialize, Serialize};
use strata_acct_types::{BitcoinAmount, Hash, SubjectId};
use strata_ee_acct_types::{
    EeAccountState, PendingFinclEntry, PendingInputEntry, MAX_PENDING_FINCLS, MAX_PENDING_INPUTS,
};
use strata_ee_chain_types::SubjectDepositData;

use crate::error::DbError;

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct DBAccountStateAtEpoch {
    epoch: u32,
    slot: u64,
    account_state: DBEeAccountState,
}

impl DBAccountStateAtEpoch {
    pub(crate) fn from_parts(epoch: u32, slot: u64, account_state: DBEeAccountState) -> Self {
        Self {
            epoch,
            slot,
            account_state,
        }
    }

    pub(crate) fn into_parts(self) -> (u32, u64, DBEeAccountState) {
        (self.epoch, self.slot, self.account_state)
    }
}

// TODO(STR-3421): Migrate EE account-state persistence away from this serde mirror and store
// the SSZ account-state type directly, including any needed DB compatibility/versioning path
// for existing local data.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct DBEeAccountState {
    #[serde(with = "serde_bytes")]
    last_exec_blkid: [u8; 32],
    #[serde(with = "serde_bytes")]
    last_exec_state_root: [u8; 32],
    pending_inputs: Vec<DBPendingInputEntry>,
    pending_fincls: Vec<DBPendingFinclEntry>,
}

impl From<EeAccountState> for DBEeAccountState {
    fn from(value: EeAccountState) -> Self {
        let (last_exec_blkid, last_exec_state_root, pending_inputs, pending_fincls) =
            value.into_parts();
        Self {
            last_exec_blkid: last_exec_blkid.into(),
            last_exec_state_root: last_exec_state_root.into(),
            pending_inputs: pending_inputs.into_iter().map(Into::into).collect(),
            pending_fincls: pending_fincls.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<DBEeAccountState> for EeAccountState {
    type Error = DbError;

    /// Rebuilds the SSZ account state, checking both list bounds first.
    ///
    /// `EeAccountState::new` panics when a list exceeds its SSZ capacity, so the bounds are
    /// enforced here: a corrupt row must surface as a decode error rather than take the node
    /// down on a read.
    fn try_from(value: DBEeAccountState) -> Result<Self, Self::Error> {
        check_list_capacity(
            "pending inputs",
            value.pending_inputs.len(),
            MAX_PENDING_INPUTS,
        )?;
        check_list_capacity(
            "pending fincls",
            value.pending_fincls.len(),
            MAX_PENDING_FINCLS,
        )?;

        Ok(Self::new(
            Hash::from(value.last_exec_blkid),
            Hash::from(value.last_exec_state_root),
            value.pending_inputs.into_iter().map(Into::into).collect(),
            value.pending_fincls.into_iter().map(Into::into).collect(),
        ))
    }
}

/// Rejects a stored list that would overflow its SSZ capacity.
fn check_list_capacity(list: &'static str, got: usize, max: u64) -> Result<(), DbError> {
    let max = max as usize;
    if got > max {
        return Err(DbError::AccountStateListOverCapacity { list, max, got });
    }
    Ok(())
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct DBPendingFinclEntry {
    epoch: u32,
    #[serde(with = "serde_bytes")]
    raw_tx_hash: [u8; 32],
}

impl From<PendingFinclEntry> for DBPendingFinclEntry {
    fn from(value: PendingFinclEntry) -> Self {
        let (epoch, raw_tx_hash) = value.into_parts();
        Self {
            epoch,
            raw_tx_hash: raw_tx_hash.into(),
        }
    }
}

impl From<DBPendingFinclEntry> for PendingFinclEntry {
    fn from(value: DBPendingFinclEntry) -> Self {
        let DBPendingFinclEntry { epoch, raw_tx_hash } = value;
        Self::new(epoch, Hash::from(raw_tx_hash))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
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

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
struct DBSubjectId(#[serde(with = "serde_bytes")] [u8; 32]);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn fincl_entry() -> DBPendingFinclEntry {
        DBPendingFinclEntry {
            epoch: 0,
            raw_tx_hash: [0u8; 32],
        }
    }

    /// An over-capacity list must not reach `EeAccountState::new`, which panics on it.
    #[test]
    fn over_capacity_fincls_is_an_error() {
        let state = DBEeAccountState {
            last_exec_blkid: [0u8; 32],
            last_exec_state_root: [0u8; 32],
            pending_inputs: Vec::new(),
            pending_fincls: vec![fincl_entry(); MAX_PENDING_FINCLS as usize + 1],
        };

        assert!(matches!(
            EeAccountState::try_from(state),
            Err(DbError::AccountStateListOverCapacity {
                list: "pending fincls",
                ..
            })
        ));
    }

    #[test]
    fn within_capacity_roundtrips() {
        let state = DBEeAccountState {
            last_exec_blkid: [1u8; 32],
            last_exec_state_root: [2u8; 32],
            pending_inputs: Vec::new(),
            pending_fincls: vec![fincl_entry()],
        };

        let converted = EeAccountState::try_from(state.clone()).expect("within capacity");
        assert_eq!(DBEeAccountState::from(converted), state);
    }

    /// Hashes must reach CBOR as byte strings, not sequences of integers.
    #[test]
    fn cbor_encodes_hashes_as_byte_strings() {
        let state = DBEeAccountState {
            last_exec_blkid: [1u8; 32],
            last_exec_state_root: [2u8; 32],
            pending_inputs: Vec::new(),
            pending_fincls: Vec::new(),
        };

        let mut encoded = Vec::new();
        ciborium::into_writer(&state, &mut encoded).unwrap();

        // `0x58 0x20` is the CBOR header for a 32-byte byte string. Without `serde_bytes` the
        // hashes would encode as arrays of integers (header `0x98 0x20`) instead, roughly
        // doubling their cost. Assert the header directly rather than guessing at a size
        // bound, since the field names dominate the total for a small record like this.
        assert!(
            encoded.windows(2).any(|pair| pair == [0x58, 0x20]),
            "expected a 32-byte CBOR byte string, got {encoded:02x?}"
        );

        let decoded: DBEeAccountState = ciborium::from_reader(encoded.as_slice()).unwrap();
        assert_eq!(decoded, state);
    }
}
