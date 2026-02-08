use alpen_ee_common::ExecBlockRecord;
use ssz::{Decode, Encode};
use strata_acct_types::{BitcoinAmount, Hash, MsgPayload};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::OLBlockCommitment;
use strata_snark_acct_types::MessageEntry;

use super::account_state::DBEeAccountState;

#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct DBExecBlockRecord {
    pub(crate) blocknum: u64,
    parent_blockhash: Hash,
    timestamp_ms: u64,
    ol_block: OLBlockCommitment,
    /// ExecBlockPackage serialized using SSZ, then stored as raw bytes.
    package_ssz: Vec<u8>,
    account_state: DBEeAccountState,
    next_inbox_msg_idx: u64,
    messages: Vec<DBMessageEntry>,
}

impl From<ExecBlockRecord> for DBExecBlockRecord {
    fn from(value: ExecBlockRecord) -> Self {
        let blocknum = value.blocknum();
        let parent_blockhash = value.parent_blockhash();
        let timestamp_ms = value.timestamp_ms();
        let ol_block = *value.ol_block();
        let next_inbox_msg_idx = value.next_inbox_msg_idx();
        let (package, account_state, messages) = value.into_parts();
        let package_ssz = package.as_ssz_bytes();
        let account_state = account_state.into();
        let messages = messages.into_iter().map(Into::into).collect();

        Self {
            blocknum,
            parent_blockhash,
            timestamp_ms,
            ol_block,
            package_ssz,
            account_state,
            next_inbox_msg_idx,
            messages,
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
            value.next_inbox_msg_idx,
            value.messages.into_iter().map(Into::into).collect(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct DBMessageEntry {
    source: [u8; 32],
    incl_epoch: u32,
    payload_value_sats: u64,
    payload_data: Vec<u8>,
}

impl From<MessageEntry> for DBMessageEntry {
    fn from(value: MessageEntry) -> Self {
        DBMessageEntry {
            source: value.source.into_inner(),
            incl_epoch: value.incl_epoch,
            payload_value_sats: value.payload().value().to_sat(),
            payload_data: value.payload().data.to_vec(),
        }
    }
}

impl From<DBMessageEntry> for MessageEntry {
    fn from(value: DBMessageEntry) -> Self {
        MessageEntry::new(
            value.source.into(),
            value.incl_epoch,
            MsgPayload::new(
                BitcoinAmount::from_sat(value.payload_value_sats),
                value.payload_data,
            ),
        )
    }
}
