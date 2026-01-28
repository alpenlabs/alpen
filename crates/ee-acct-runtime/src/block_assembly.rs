//! Helpers for working with ee account during block assembly.

use strata_ee_acct_types::{DecodedEeMessageData, EeAccountState, EnvError, EnvResult};
use strata_snark_acct_runtime::{InputMessage, MsgMeta};
use strata_snark_acct_types::MessageEntry;

use crate::ee_program::apply_decoded_message;

/// Applies state changes from list of messages.
///
/// Returns the successfully parsed messages along with their metadata.
/// Unknown/unparseable messages are skipped.
pub fn apply_input_messages(
    astate: &mut EeAccountState,
    msgs: &[MessageEntry],
) -> EnvResult<Vec<InputMessage<DecodedEeMessageData>>> {
    let mut parsed_messages = Vec::with_capacity(msgs.len());

    for entry in msgs.iter() {
        let meta = MsgMeta::new(entry.source(), entry.incl_epoch(), entry.payload_value());

        // Try to decode the message; skip if it fails.
        let Ok(decoded) = DecodedEeMessageData::decode_raw(entry.payload_buf()) else {
            continue;
        };

        // Add value to tracked balance.
        if !meta.value().is_zero() {
            astate.add_tracked_balance(meta.value());
        }

        // Apply the decoded message effects.
        // Errors here indicate internal issues since we successfully decoded.
        apply_decoded_message(astate, &decoded, meta.value()).map_err(|_| EnvError::InvalidBlock)?;

        parsed_messages.push(InputMessage::Valid(meta, decoded));
    }

    Ok(parsed_messages)
}
