use strata_acct_types::AcctSerial;
use strata_ee_acct_types::EeAccountState;

/// Ledger account state
#[derive(Debug, Clone)]
pub struct AccountState {
    pub serial: AcctSerial,
    /// Account type
    pub ty: u16, // Maybe a separate type
    pub balance: u64, // sats
    pub inner_state: AccountInnerState,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AccountInnerState {
    Snark(EeAccountState),  // From ee-acct-types that you created
    // add others
}
