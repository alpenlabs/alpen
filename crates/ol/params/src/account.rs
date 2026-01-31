//! Genesis account parameters.

use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;
use strata_predicate::PredicateKey;

/// Parameters for a single genesis snark account.
///
/// The `predicate` and `inner_state` fields are required. The `balance` field
/// defaults to 0 if omitted. Other account fields (`serial`, `seqno`,
/// `inbox_mmr`, `next_msg_read_idx`) are auto-computed at genesis construction
/// time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountParams {
    /// Verifying key (predicate).
    pub predicate: PredicateKey,

    /// Inner state root commitment.
    pub inner_state: Buf32,

    /// Initial balance in satoshis. Defaults to 0.
    #[serde(default)]
    pub balance: u64,
}
