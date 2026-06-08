pub use strata_identifiers::SYSTEM_RESERVED_ACCTS;
use strata_identifiers::{AccountId, AccountSerial};

const BRIDGE_GATEWAY_REF: u8 = 0x10;

/// Account ID that we use for the bridge gateway account.
pub const BRIDGE_GATEWAY_ACCT_ID: AccountId = AccountId::special(BRIDGE_GATEWAY_REF);

/// Serial of the bridge gateway account.
pub const BRIDGE_GATEWAY_ACCT_SERIAL: AccountSerial = AccountSerial::reserved(BRIDGE_GATEWAY_REF);
