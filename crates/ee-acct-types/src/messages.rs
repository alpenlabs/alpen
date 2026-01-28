//! Definitions for EE message types.

use strata_acct_types::SubjectId;
use strata_codec::{Codec, VarVec, decode_buf_exact, impl_type_flat_struct};
use strata_msg_fmt::{Msg, MsgRef, TypeId};
use strata_snark_acct_runtime::IAcctMsg;

use crate::{MessageDecodeError, MessageDecodeResult};

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
                let data = decode_codec_msg_body::<DepositMsgData>(body)?;
                Ok(DecodedEeMessageData::Deposit(data))
            }

            SUBJ_TRANSFER_MSG_TYPE => {
                let data = decode_codec_msg_body::<SubjTransferMsgData>(body)?;
                Ok(DecodedEeMessageData::SubjTransfer(data))
            }

            COMMIT_MSG_TYPE => {
                let data = decode_codec_msg_body::<CommitMsgData>(body)?;
                Ok(DecodedEeMessageData::Commit(data))
            }

            ty => Err(MessageDecodeError::UnsupportedType(ty)),
        }
    }
}

impl IAcctMsg for DecodedEeMessageData {
    type ParseError = MessageDecodeError;

    fn try_parse(buf: &[u8]) -> Result<Self, Self::ParseError> {
        Self::decode_raw(buf)
    }
}

/// Decode a message body from a buffer.
fn decode_codec_msg_body<T: Codec>(buf: &[u8]) -> MessageDecodeResult<T> {
    decode_buf_exact(buf).map_err(|_| MessageDecodeError::InvalidBody)
}

impl_type_flat_struct! {
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct DepositMsgData {
        dest_subject: SubjectId,
    }
}

impl_type_flat_struct! {
    /// Describes a transfer between subjects in EEs.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct SubjTransferMsgData {
        source_subject: SubjectId,
        dest_subject: SubjectId,
        transfer_data: VarVec<u8>,
    }
}

impl SubjTransferMsgData {
    pub fn data_buf(&self) -> &[u8] {
        self.transfer_data().as_slice()
    }
}

impl_type_flat_struct! {
    /// Describes a chunk a sequencer wants to stage.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CommitMsgData {
        // TODO rename to new_tip_exec_blkid
        new_tip_exec_blkid: [u8; 32],
    }
}
