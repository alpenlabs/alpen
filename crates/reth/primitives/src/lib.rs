//! Primitives for Reth.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_sol_types::sol;
use serde::{Deserialize, Serialize};
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32};

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

    /// User's selected operator index for withdrawal assignment.
    /// `u32::MAX` means no selection (random assignment).
    pub selected_operator: u32,
}

sol! {
    event WithdrawalIntentEvent(
        /// Withdrawal amount in sats.
        uint64 amount,
        /// BOSD descriptor for withdrawal destinations in L1.
        bytes destination,
        /// Selected operator index. `u32::MAX` means no selection.
        uint32 selectedOperator,
    );
}
