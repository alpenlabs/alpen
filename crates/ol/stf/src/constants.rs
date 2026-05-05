use strata_acct_types::SYSTEM_RESERVED_ACCTS;
use strata_identifiers::{AccountId, AccountSerial};

const BRIDGE_GATEWAY_REF: u8 = 0x10;

/// Account ID that we use for the bridge gateway account.
pub const BRIDGE_GATEWAY_ACCT_ID: AccountId = AccountId::special(BRIDGE_GATEWAY_REF);

/// Serial of the bridge gateway account.
pub const BRIDGE_GATEWAY_ACCT_SERIAL: AccountSerial = AccountSerial::reserved(BRIDGE_GATEWAY_REF);

/// ID for sequencer-sent accounts.
// TODO make this different, really, it should be the sequencer producing the block
pub const SEQUENCER_ACCT_ID: AccountId = BRIDGE_GATEWAY_ACCT_ID;

/// Serial of the bridge gateway account.
// TODO make this different, really, it should be the sequencer producing the block
pub const SEQUENCER_ACCT_SERIAL: AccountSerial = BRIDGE_GATEWAY_ACCT_SERIAL;

/// Serial of the Alpen EE snark account.
///
/// System serials occupy `0..SYSTEM_RESERVED_ACCTS`, so the Alpen EE
/// account, registered first at genesis, lands at serial
/// `SYSTEM_RESERVED_ACCTS` (currently 128).
// TODO(STR-2021): query this from OL params or via RPC at runtime.
pub const ALPEN_EE_ACCT_SERIAL: AccountSerial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);
