//! Helpers for working with ee account during block assembly.

use strata_ee_acct_types::{DecodedEeMessageData, EeAccountState, EnvError, EnvResult};
use strata_snark_acct_runtime::InputMessage;
use strata_snark_acct_types::MessageEntry;

use crate::ee_program::process_input_message;

/// Applies state changes from a list of messages.
///
/// Returns all parsed messages, including unknown/unparsable ones (which will
/// have `message() == None`). Value is always tracked regardless of whether the
/// message was successfully decoded.
pub fn apply_input_messages(
    astate: &mut EeAccountState,
    msgs: &[MessageEntry],
) -> EnvResult<Vec<InputMessage<DecodedEeMessageData>>> {
    let mut parsed_messages = Vec::with_capacity(msgs.len());

    for entry in msgs.iter() {
        let input_msg = InputMessage::from_msg_entry(entry);

        process_input_message(astate, &input_msg).map_err(|_| EnvError::InvalidBlock)?;

        parsed_messages.push(input_msg);
    }

    Ok(parsed_messages)
}
