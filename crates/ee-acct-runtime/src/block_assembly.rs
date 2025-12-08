//! Helpers for working with ee account during block assembly.

use strata_ee_acct_types::{EeAccountState, EnvResult};
use strata_snark_acct_types::MessageEntry;

use crate::update_processing::{MsgData, apply_message, make_inp_err_indexer};

/// Applies state changes from list of messages.
pub fn apply_input_messages(
    astate: &mut EeAccountState,
    msgs: &[MessageEntry],
) -> EnvResult<Vec<MsgData>> {
    let mut parsed_messages = Vec::with_capacity(msgs.len());
    for (i, inp) in msgs.iter().enumerate() {
        let Some(msg) = MsgData::from_entry(inp).ok() else {
            continue;
        };

        apply_message(astate, &msg).map_err(make_inp_err_indexer(i))?;

        parsed_messages.push(msg);
    }

    Ok(parsed_messages)
}
