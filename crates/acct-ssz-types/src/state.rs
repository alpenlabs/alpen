// Account state types from the SSZ schema.
// Types defined here match the pythonic schema in `schemas/acct-types.ssz`.

use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use tree_hash_derive::TreeHash as TreeHashDerive;

use crate::{AccountSerial, BitcoinAmount, MAX_ACCOUNT_ENCODED_STATE_BYTES};

/// Variable-length byte list for account encoded state
pub type AccountEncodedState = VariableList<u8, MAX_ACCOUNT_ENCODED_STATE_BYTES>;

/// Intrinsic account state (core state shared by all accounts)
/// Schema: class IntrinsicAccountState(Container)
#[derive(Copy, Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct IntrinsicAccountState {
    pub raw_ty: u16,
    pub serial: AccountSerial,
    pub balance: BitcoinAmount,
}

/// Account state with type-specific encoded data
/// Schema: class AccountState(Container)
#[derive(Clone, Debug, Encode, Decode, TreeHashDerive)]
pub struct AccountState {
    pub intrinsics: IntrinsicAccountState,
    pub encoded_state: AccountEncodedState,
}

/// Account state summary for merkle proofs
/// Schema: class AcctStateSummary(Container)
#[derive(Copy, Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct AcctStateSummary {
    pub serial: AccountSerial,
    pub balance: BitcoinAmount,
    pub state_root: [u8; 32],
}
