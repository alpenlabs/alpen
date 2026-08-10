use alpen_ee_common::ExecBlockRecord;
use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use strata_acct_types::{BitcoinAmount, Hash, MessageEntry, MsgPayload};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId, Slot};

use super::account_state::DBEeAccountState;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct DBExecBlockRecord {
    pub(crate) blocknum: u64,

    // TODO(db-refactor-part-17): mirror field pending upstream Buf32 serde fix
    parent_blockhash: [u8; 32],

    timestamp_ms: u64,

    /// Slot of the OL block this record is anchored to.
    ol_block_slot: Slot,

    // TODO(db-refactor-part-17): mirror field pending upstream Buf32 serde fix
    /// Id of the OL block this record is anchored to.
    ol_block_id: [u8; 32],

    /// ExecBlockPackage serialized using SSZ, stored as opaque bytes.
    // TODO(db-refactor-part-1): store the SSZ value directly via a SerdeSsz wrapper
    package_ssz: Vec<u8>,
    account_state: DBEeAccountState,
    next_inbox_msg_idx: u64,
    next_deposit_idx: u64,
    messages: Vec<DBMessageEntry>,
}

impl From<ExecBlockRecord> for DBExecBlockRecord {
    fn from(value: ExecBlockRecord) -> Self {
        let blocknum = value.blocknum();
        let parent_blockhash = value.parent_blockhash().into();
        let timestamp_ms = value.timestamp_ms();
        let ol_block = *value.ol_block();
        let ol_block_slot = ol_block.slot();
        let ol_block_id = Buf32::from(*ol_block.blkid()).into();
        let next_inbox_msg_idx = value.next_inbox_msg_idx();
        let next_deposit_idx = value.next_deposit_idx();
        let (package, account_state, messages) = value.into_parts();
        let package_ssz = package.as_ssz_bytes();
        let account_state = account_state.into();
        let messages = messages.into_iter().map(Into::into).collect();

        Self {
            blocknum,
            parent_blockhash,
            timestamp_ms,
            ol_block_slot,
            ol_block_id,
            package_ssz,
            account_state,
            next_inbox_msg_idx,
            next_deposit_idx,
            messages,
        }
    }
}

impl TryFrom<DBExecBlockRecord> for ExecBlockRecord {
    type Error = ssz::DecodeError;

    fn try_from(value: DBExecBlockRecord) -> Result<Self, Self::Error> {
        let package = ExecBlockPackage::from_ssz_bytes(&value.package_ssz)?;
        let account_state: EeAccountState = value.account_state.into();

        let ol_block = OLBlockCommitment::new(
            value.ol_block_slot,
            OLBlockId::from(Buf32::from(value.ol_block_id)),
        );

        Ok(ExecBlockRecord::new(
            package,
            account_state,
            value.blocknum,
            ol_block,
            value.timestamp_ms,
            Hash::from(value.parent_blockhash),
            value.next_inbox_msg_idx,
            value.next_deposit_idx,
            value.messages.into_iter().map(Into::into).collect(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
            MsgPayload::from_bytes(
                BitcoinAmount::from_sat(value.payload_value_sats),
                value.payload_data,
            )
            .expect("database message payload bytes must fit within SSZ max length"),
        )
    }
}
