//! Message parsing utilities for orchestration layer messages.
// TODO this is some weird thing claude invented, I've seen it before but it seems tedious to keep
// maintained, maybe we should get rid of it?

use strata_codec::decode_buf_exact;
use strata_msg_fmt::{Msg, MsgRef};

use crate::{
    DEPOSIT_MSG_TYPE_ID, DepositMsgData, WITHDRAWAL_FEE_BUMP_MSG_TYPE_ID, WITHDRAWAL_MSG_TYPE_ID,
    WITHDRAWAL_REJECTION_MSG_TYPE_ID, WithdrawalFeeBumpMsgData, WithdrawalMsgData,
    WithdrawalRejectionMsgData,
};

/// Helper function to decode a message body with proper error handling.
fn decode_msg_body<T: strata_codec::Codec>(body: &[u8]) -> Option<T> {
    decode_buf_exact(body).ok()
}

/// Extension trait for MsgRef to decode OL-specific message types.
pub trait OLMessageExt {
    /// Try to decode as a deposit message.
    fn try_as_deposit(&self) -> Option<DepositMsgData>;

    /// Try to decode as a withdrawal message.
    fn try_as_withdrawal(&self) -> Option<WithdrawalMsgData>;

    /// Try to decode as a withdrawal fee bump message.
    fn try_as_withdrawal_fee_bump(&self) -> Option<WithdrawalFeeBumpMsgData>;

    /// Try to decode as a withdrawal rejection message.
    fn try_as_withdrawal_rejection(&self) -> Option<WithdrawalRejectionMsgData>;
}

impl<'a> OLMessageExt for MsgRef<'a> {
    fn try_as_deposit(&self) -> Option<DepositMsgData> {
        if self.ty() != DEPOSIT_MSG_TYPE_ID {
            return None;
        }
        decode_msg_body(self.body())
    }

    fn try_as_withdrawal(&self) -> Option<WithdrawalMsgData> {
        if self.ty() != WITHDRAWAL_MSG_TYPE_ID {
            return None;
        }
        decode_msg_body(self.body())
    }

    fn try_as_withdrawal_fee_bump(&self) -> Option<WithdrawalFeeBumpMsgData> {
        if self.ty() != WITHDRAWAL_FEE_BUMP_MSG_TYPE_ID {
            return None;
        }
        decode_msg_body(self.body())
    }

    fn try_as_withdrawal_rejection(&self) -> Option<WithdrawalRejectionMsgData> {
        if self.ty() != WITHDRAWAL_REJECTION_MSG_TYPE_ID {
            return None;
        }
        decode_msg_body(self.body())
    }
}
