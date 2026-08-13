use strata_acct_types::{
    AccountId, AccountSerial, BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL,
};

/// ID for sequencer-sent accounts.
// TODO(STR-3677): make this different, really, it should be the sequencer producing the block
pub const SEQUENCER_ACCT_ID: AccountId = BRIDGE_GATEWAY_ACCT_ID;

/// Serial of the bridge gateway account.
// TODO(STR-3677): make this different, really, it should be the sequencer producing the block
pub const SEQUENCER_ACCT_SERIAL: AccountSerial = BRIDGE_GATEWAY_ACCT_SERIAL;
