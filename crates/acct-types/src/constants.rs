use strata_identifiers::AccountId;
pub use strata_identifiers::SYSTEM_RESERVED_ACCTS;

const ADMIN_MSG_REF: u8 = 0x01;

/// Account ID used as the source of system messages emitted by admin
/// actions (e.g. predicate key rotations). Reserved; no ledger account can
/// occupy it.
pub const ADMIN_MSG_ACCT_ID: AccountId = AccountId::special(ADMIN_MSG_REF);
