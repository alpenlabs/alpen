//! Withdrawal message parsing and validation for the bridge gateway.
//!
//! STF execution and transaction admission use these checks to agree on which
//! messages produce withdrawal logs.

use bitcoin_bosd::Descriptor;
use strata_msg_fmt::MsgRef;
use strata_ol_chain_types_v1::SimpleWithdrawalIntentLogData;
use strata_ol_msg_types::OLMessageExt;
use strata_ol_params::BridgeParams;

/// Reason a bridge gateway message sends its value to limbo instead of withdrawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeMessageRejection {
    /// Invalid message format, sweep to limbo.
    MalformedMessage,
    /// Not a withdrawal message, or malformed, sweep to limbo.
    NotWithdrawal,
    InvalidAmount,
    InvalidDescriptor {
        len: usize,
    },
}

/// Parses a withdrawal message and validates its amount and destination.
///
/// `data` contains the encoded message; `amount` is its attached value in satoshis.
/// Requires bridge parameters, a valid withdrawal amount, and a valid BOSD
/// destination within the configured descriptor length limit.
///
/// Returns withdrawal log data on success or a [`BridgeMessageRejection`] on
/// failure. STF execution sends rejected messages' funds to limbo; transaction
/// admission counts no withdrawal log for them.
///
/// The caller must check that the message targets the bridge gateway first.
pub fn parse_bridge_withdrawal(
    amount: u64,
    data: &[u8],
    bridge_params: &BridgeParams,
) -> Result<SimpleWithdrawalIntentLogData, BridgeMessageRejection> {
    // 1. Parse the message from the payload data.
    let message = MsgRef::try_from(data).map_err(|_| BridgeMessageRejection::MalformedMessage)?;
    let withdrawal = message
        .try_as_withdrawal()
        .ok_or(BridgeMessageRejection::NotWithdrawal)?;

    // 2. Validate the withdrawal amount against params.
    if !bridge_params.validate_withdrawal_amount(amount) {
        return Err(BridgeMessageRejection::InvalidAmount);
    }

    // 3. Validate the withdrawal descriptor against the configured BOSD policy.
    let descriptor = withdrawal.dest_desc();
    if !bridge_params.validate_withdrawal_descriptor_len(descriptor.len())
        || Descriptor::from_bytes(descriptor).is_err()
    {
        return Err(BridgeMessageRejection::InvalidDescriptor {
            len: descriptor.len(),
        });
    }

    Ok(SimpleWithdrawalIntentLogData {
        amt: amount,
        selected_operator: withdrawal.selected_operator(),
        dest: withdrawal.into_dest_desc(),
    })
}
