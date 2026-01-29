use std::sync::LazyLock;

use strata_identifiers::{AccountId, AccountSerial};

const BRIDGE_GATEWAY_REF: u8 = 0x10;

/// Account ID that we use for the bridge gateway account.
pub const BRIDGE_GATEWAY_ACCT_ID: AccountId = AccountId::special(BRIDGE_GATEWAY_REF);

/// Serial of the bridge gateway account.
pub static BRIDGE_GATEWAY_ACCT_SERIAL: LazyLock<AccountSerial> =
    LazyLock::new(|| AccountSerial::reserved(BRIDGE_GATEWAY_REF));

/// ID for sequencer-sent accounts.
// TODO make this different, really, it should be the sequencer producing the block
pub const SEQUENCER_ACCT_ID: AccountId = BRIDGE_GATEWAY_ACCT_ID;

/// Serial of the bridge gateway account.
// TODO make this different, really, it should be the sequencer producing the block
pub static SEQUENCER_ACCT_SERIAL: LazyLock<AccountSerial> =
    LazyLock::new(|| *BRIDGE_GATEWAY_ACCT_SERIAL);
