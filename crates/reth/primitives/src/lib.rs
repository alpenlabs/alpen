//! Primitives for Reth.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::mem::size_of;

use alloy_sol_types::sol;
use serde::{Deserialize, Serialize};
use strata_bridge_types::OperatorSelection;
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32};

/// Type for withdrawal_intents in rpc.
/// Distinct from `strata_bridge_types::WithdrawalIntent`
/// as this will live in reth repo eventually
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct WithdrawalIntent {
    /// Amount to be withdrawn in sats.
    pub amt: u64,

    /// User's operator selection for withdrawal assignment.
    pub selected_operator: OperatorSelection,

    /// Withdrawal request transaction id.
    pub withdrawal_txid: Buf32,

    /// Dynamic-sized bytes BOSD descriptor for the withdrawal destinations in L1.
    pub destination: Descriptor,
}

sol! {
    event WithdrawalIntentEvent(
        /// Withdrawal amount in sats.
        uint64 amount,
        /// Selected operator index. `u32::MAX` means no selection.
        uint32 selectedOperator,
        /// BOSD descriptor for withdrawal destinations in L1.
        bytes destination,
    );
}

/// Structured calldata for the bridge-out withdrawal precompile.
///
/// Wire format: `[4 bytes: operator index (big-endian u32)][BOSD bytes]`
/// - `u32::MAX` (`0xFFFFFFFF`): no operator selection (any operator)
/// - Any other value: specific operator index
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WithdrawalCalldata {
    /// User's operator selection for withdrawal assignment.
    pub selected_operator: OperatorSelection,

    /// Raw BOSD descriptor bytes.
    pub bosd: Vec<u8>,
}

/// Size of the operator index field in calldata (u32 = 4 bytes).
const OPERATOR_INDEX_SIZE: usize = size_of::<u32>();

impl WithdrawalCalldata {
    /// Encodes the calldata to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(OPERATOR_INDEX_SIZE + self.bosd.len());
        buf.extend_from_slice(&self.selected_operator.raw().to_be_bytes());
        buf.extend_from_slice(&self.bosd);
        buf
    }

    /// Decodes calldata from bytes.
    ///
    /// Returns `None` if the data is too short (needs at least 5 bytes: 4 operator + 1 BOSD).
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() <= OPERATOR_INDEX_SIZE {
            return None;
        }

        let (operator_bytes, bosd) = data.split_at(OPERATOR_INDEX_SIZE);
        let raw = u32::from_be_bytes(operator_bytes.try_into().expect("exactly 4 bytes"));

        Some(Self {
            selected_operator: OperatorSelection::from_raw(raw),
            bosd: bosd.to_vec(),
        })
    }
}
