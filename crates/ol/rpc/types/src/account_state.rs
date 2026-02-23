use serde::{Deserialize, Serialize};
use strata_primitives::{HexBytes, HexBytes32};
use strata_snark_acct_types::SnarkAccountState;

/// Snark account state for RPC responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcSnarkAccountState {
    /// Account sequence number.
    seq_no: u64,
    /// Merkle root of the account state.
    inner_state: HexBytes32,
    /// Index of the next inbox message to process.
    next_inbox_msg_idx: u64,
    /// Snark account update verification key
    update_vk: HexBytes,
}

impl RpcSnarkAccountState {
    /// Creates a new `RpcSnarkAccountState`.
    pub fn new(
        seq_no: u64,
        inner_state: HexBytes32,
        next_inbox_msg_idx: u64,
        update_vk: HexBytes,
    ) -> Self {
        Self {
            seq_no,
            inner_state,
            next_inbox_msg_idx,
            update_vk,
        }
    }

    /// Returns the account sequence number.
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    /// Returns the state root.
    pub fn inner_state(&self) -> &HexBytes32 {
        &self.inner_state
    }

    /// Returns the next inbox message index.
    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }
}

impl From<SnarkAccountState> for RpcSnarkAccountState {
    fn from(value: SnarkAccountState) -> Self {
        Self {
            seq_no: value.seq_no,
            inner_state: value.proof_state().inner_state().0.into(),
            next_inbox_msg_idx: value.proof_state().next_inbox_msg_idx(),
            update_vk: value.update_vk().to_vec().into(),
        }
    }
}
