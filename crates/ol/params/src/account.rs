//! Genesis account parameters.

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use strata_btc_types::BitcoinAmount;
use strata_identifiers::Buf32;
use strata_predicate::PredicateKey;

/// Parameters for a single genesis snark account.
///
/// The `predicate` and `inner_state` fields are required. The `balance` field
/// defaults to [`BitcoinAmount::ZERO`] if omitted.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct AccountParams {
    /// Verifying key (predicate).
    pub predicate: PredicateKey,

    /// Inner state root commitment.
    pub inner_state: Buf32,

    /// Initial balance as a [`BitcoinAmount`]. Defaults to [`BitcoinAmount::ZERO`].
    #[serde(default = "BitcoinAmount::zero")]
    pub balance: BitcoinAmount,
}
