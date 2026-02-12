//! Primitives for Reth.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_sol_types::sol;
use serde::{Deserialize, Serialize};
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32};

/// Sentinel value indicating no preferred operator for withdrawal assignment.
pub const NO_PREFERRED_OPERATOR: u32 = u32::MAX;

/// Type for withdrawal_intents in rpc.
/// Distinct from `strata_bridge_types::WithdrawalIntent`
/// as this will live in reth repo eventually
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct WithdrawalIntent {
    /// Amount to be withdrawn in sats.
    pub amt: u64,

    /// Dynamic-sized bytes BOSD descriptor for the withdrawal destinations in L1.
    pub destination: Descriptor,

    /// withdrawal request transaction id
    pub withdrawal_txid: Buf32,

    /// User's preferred operator index for withdrawal assignment.
    /// [`NO_PREFERRED_OPERATOR`] means no preference (random assignment).
    pub preferred_operator: u32,
}

sol! {
    event WithdrawalIntentEvent(
        /// Withdrawal amount in sats.
        uint64 amount,
        /// BOSD descriptor for withdrawal destinations in L1.
        bytes destination,
        /// Preferred operator index. `u32::MAX` means no preference.
        uint32 preferredOperator,
    );
}
