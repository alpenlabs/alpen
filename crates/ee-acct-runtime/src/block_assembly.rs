//! Helpers for working with ee account during block assembly.

use strata_acct_types::MessageEntry;
use strata_ee_acct_types::{EeAccountState, EnvError, EnvResult};
use strata_snark_acct_runtime::InputMessage;

use crate::ee_program::process_input_message;

/// Applies state changes from a list of messages.
pub fn apply_input_messages(astate: &mut EeAccountState, msgs: &[MessageEntry]) -> EnvResult<()> {
    for entry in msgs.iter() {
        let input_msg = InputMessage::from_msg_entry(entry);

        process_input_message(astate, &input_msg).map_err(|_| EnvError::InvalidBlock)?;
    }

    Ok(())
}
