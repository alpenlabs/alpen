use alpen_ee_common::ExecBlockRecord;
use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use strata_acct_types::{BitcoinAmount, Hash, MessageEntry, MsgPayload};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId, Slot};

use super::account_state::DBEeAccountState;
use crate::error::DbError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct DBExecBlockRecord {
    pub(crate) blocknum: u64,

    #[serde(with = "serde_bytes")]
    parent_blockhash: [u8; 32],

    timestamp_ms: u64,

    /// Slot of the OL block this record is anchored to.
    ol_block_slot: Slot,

    /// Id of the OL block this record is anchored to.
    #[serde(with = "serde_bytes")]
    ol_block_id: [u8; 32],

    /// ExecBlockPackage serialized using SSZ, stored as opaque bytes.
    // TODO(db-refactor-part-1): store the SSZ value directly via a SerdeSsz wrapper
    #[serde(with = "serde_bytes")]
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
    type Error = DbError;

    fn try_from(value: DBExecBlockRecord) -> Result<Self, Self::Error> {
        let package = ExecBlockPackage::from_ssz_bytes(&value.package_ssz)
            .map_err(|err| DbError::ExecBlockDeserialize(format!("{err:?}")))?;
        let account_state: EeAccountState = value.account_state.try_into()?;
        let messages = value
            .messages
            .into_iter()
            .map(MessageEntry::try_from)
            .collect::<Result<Vec<_>, _>>()?;

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
            messages,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
struct DBMessageEntry {
    #[serde(with = "serde_bytes")]
    source: [u8; 32],
    incl_epoch: u32,
    payload_value_sats: u64,
    #[serde(with = "serde_bytes")]
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

impl TryFrom<DBMessageEntry> for MessageEntry {
    type Error = DbError;

    /// Rebuilds the message entry, checking the stored payload against its SSZ bound.
    ///
    /// Only a corrupt row can exceed it, but that must not panic the node on a read.
    fn try_from(value: DBMessageEntry) -> Result<Self, Self::Error> {
        let payload_len = value.payload_data.len();
        let payload = MsgPayload::from_bytes(
            BitcoinAmount::from_sat(value.payload_value_sats),
            value.payload_data,
        )
        .map_err(|_| DbError::MessagePayloadOverCapacity(payload_len))?;

        Ok(MessageEntry::new(
            value.source.into(),
            value.incl_epoch,
            payload,
        ))
    }
}
