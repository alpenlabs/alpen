//! Definitions for EE message types.

use ssz::Decode;
use ssz_types::VariableList;
use strata_acct_types::SubjectId;
use strata_msg_fmt::{Msg, MsgRef, TypeId};

use crate::{
    MessageDecodeError, MessageDecodeResult,
    ssz_generated::ssz::messages::{CommitMsgData, DepositMsgData, SubjTransferMsgData},
};

type TransferData = VariableList<u8, 1048576>;

/// Message type ID for deposit messages.
pub const DEPOSIT_MSG_TYPE: TypeId = 0x02;

/// Message type ID for subject transfer messages.
pub const SUBJ_TRANSFER_MSG_TYPE: TypeId = 0x01;

/// Message type ID for commit messages.
pub const COMMIT_MSG_TYPE: TypeId = 0x10;

/// Decoded possible EE account messages we want to honor.
///
/// This is not intended to capture all possible message types.
// TODO make zero copy?
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedEeMessageData {
    /// Deposit from L1 to a subject in the EE.
    Deposit(DepositMsgData),

    /// Transfer from a subject in one EE to a subject in another EE.
    SubjTransfer(SubjTransferMsgData),

    /// Commit an update.
    Commit(CommitMsgData),
}

impl DecodedEeMessageData {
    /// Decode a raw message buffer, distinguishing its type.
    pub fn decode_raw(buf: &[u8]) -> MessageDecodeResult<DecodedEeMessageData> {
        let msg = MsgRef::try_from(buf).map_err(|_| MessageDecodeError::InvalidFormat)?;
        let body = msg.body();

        match msg.ty() {
            DEPOSIT_MSG_TYPE => {
                let data = decode_ssz_msg_body::<DepositMsgData>(body)?;
                Ok(DecodedEeMessageData::Deposit(data))
            }

            SUBJ_TRANSFER_MSG_TYPE => {
                let data = decode_ssz_msg_body::<SubjTransferMsgData>(body)?;
                Ok(DecodedEeMessageData::SubjTransfer(data))
            }

            COMMIT_MSG_TYPE => {
                let data = decode_ssz_msg_body::<CommitMsgData>(body)?;
                Ok(DecodedEeMessageData::Commit(data))
            }

            ty => Err(MessageDecodeError::UnsupportedType(ty)),
        }
    }
}

/// Decode a message body from a buffer.
fn decode_ssz_msg_body<T: Decode>(buf: &[u8]) -> MessageDecodeResult<T> {
    T::from_ssz_bytes(buf).map_err(|_| MessageDecodeError::InvalidBody)
}

impl DepositMsgData {
    pub fn dest_subject(&self) -> &SubjectId {
        &self.dest_subject
    }
}

impl SubjTransferMsgData {
    pub fn source_subject(&self) -> &SubjectId {
        &self.source_subject
    }

    pub fn dest_subject(&self) -> &SubjectId {
        &self.dest_subject
    }

    pub fn transfer_data(&self) -> &TransferData {
        &self.transfer_data
    }

    pub fn data_buf(&self) -> &[u8] {
        &self.transfer_data[..]
    }
}

// CommitMsgData is now generated from ssz/messages.ssz

impl CommitMsgData {
    pub fn new_tip_exec_blkid(&self) -> &[u8; 32] {
        self.new_tip_exec_blkid
            .as_ref()
            .try_into()
            .expect("FixedBytes<32> should convert to &[u8; 32]")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_acct_types::SubjectId;
    use strata_test_utils_ssz::ssz_proptest;

    use crate::ssz_generated::ssz::messages::{CommitMsgData, DepositMsgData, SubjTransferMsgData};

    mod deposit_msg_data {
        use super::*;

        ssz_proptest!(
            DepositMsgData,
            any::<[u8; 32]>().prop_map(|subject_bytes| DepositMsgData {
                dest_subject: SubjectId::from(subject_bytes),
            })
        );
    }

    mod subj_transfer_msg_data {
        use super::*;

        ssz_proptest!(
            SubjTransferMsgData,
            (
                any::<[u8; 32]>(),
                any::<[u8; 32]>(),
                prop::collection::vec(any::<u8>(), 0..256),
            )
                .prop_map(|(source_bytes, dest_bytes, data)| SubjTransferMsgData {
                    source_subject: SubjectId::from(source_bytes),
                    dest_subject: SubjectId::from(dest_bytes),
                    transfer_data: data.into(),
                })
        );
    }

    mod commit_msg_data {
        use super::*;

        ssz_proptest!(
            CommitMsgData,
            any::<[u8; 32]>().prop_map(|blkid| CommitMsgData {
                new_tip_exec_blkid: blkid.into(),
            })
        );
    }
}
