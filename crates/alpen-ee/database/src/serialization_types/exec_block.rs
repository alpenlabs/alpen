use alpen_ee_common::ExecBlockRecord;
use borsh::{BorshDeserialize, BorshSerialize};
use ssz::{Decode, Encode};
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::OLBlockCommitment;

use super::account_state::DBEeAccountState;

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBExecBlockRecord {
    pub(crate) blocknum: u64,
    parent_blockhash: Hash,
    timestamp_ms: u64,
    ol_block: OLBlockCommitment,
    /// ExecBlockPackage serialized using SSZ, then wrapped in a Vec<u8> for Borsh
    package_ssz: Vec<u8>,
    account_state: DBEeAccountState,
}

impl From<ExecBlockRecord> for DBExecBlockRecord {
    fn from(value: ExecBlockRecord) -> Self {
        let blocknum = value.blocknum();
        let parent_blockhash = value.parent_blockhash();
        let timestamp_ms = value.timestamp_ms();
        let ol_block = *value.ol_block();
        let (package, account_state) = value.into_parts();
        let package_ssz = package.as_ssz_bytes();
        let account_state = account_state.into();

        Self {
            blocknum,
            parent_blockhash,
            timestamp_ms,
            ol_block,
            package_ssz,
            account_state,
        }
    }
}

impl TryFrom<DBExecBlockRecord> for ExecBlockRecord {
    type Error = ssz::DecodeError;

    fn try_from(value: DBExecBlockRecord) -> Result<Self, Self::Error> {
        let package = ExecBlockPackage::from_ssz_bytes(&value.package_ssz)?;
        let account_state: EeAccountState = value.account_state.into();

        Ok(ExecBlockRecord::new(
            package,
            account_state,
            value.blocknum,
            value.ol_block,
            value.timestamp_ms,
            value.parent_blockhash,
        ))
    }
}
