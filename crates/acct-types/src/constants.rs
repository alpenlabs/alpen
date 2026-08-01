pub use strata_identifiers::SYSTEM_RESERVED_ACCTS;
use strata_identifiers::{AccountId, AccountSerial};

const BRIDGE_GATEWAY_REF: u8 = 0x10;

/// Account ID that we use for the bridge gateway account.
pub const BRIDGE_GATEWAY_ACCT_ID: AccountId = AccountId::special(BRIDGE_GATEWAY_REF);

/// Serial of the bridge gateway account.
pub const BRIDGE_GATEWAY_ACCT_SERIAL: AccountSerial = AccountSerial::reserved(BRIDGE_GATEWAY_REF);

const ADMIN_MSG_REF: u8 = 0x01;

/// Account ID used as the source of system messages emitted by admin
/// actions (e.g. predicate key rotations). Reserved; no ledger account can
/// occupy it.
pub const ADMIN_MSG_ACCT_ID: AccountId = AccountId::special(ADMIN_MSG_REF);
