//! Bridge state types.
//!
//! This just implements a very simple n-of-n multisig bridge.  It will be
//! extended to a more sophisticated design when we have that specced out.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::state::{assignment::AssignmentTable, deposit::DepositsTable, operator::OperatorTable};

pub mod assignment;
pub mod deposit;
pub mod deposit_state;
pub mod operator;
pub mod withdrawal;

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1State {
    operators: OperatorTable,
    deposits: DepositsTable,
    assignments: AssignmentTable,
}

impl Default for BridgeV1State {
    fn default() -> Self {
        Self {
            operators: OperatorTable::new_empty(),
            deposits: DepositsTable::new_empty(),
            assignments: AssignmentTable::new_empty(),
        }
    }
}
