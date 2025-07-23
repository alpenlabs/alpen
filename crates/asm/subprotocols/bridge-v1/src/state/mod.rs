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

/// Main state container for the Bridge V1 subprotocol.
///
/// This structure holds all the persistent state for the bridge, including
/// operator registrations, deposit tracking, and assignment management.
///
/// # Fields
///
/// - `operators` - Table of registered bridge operators with their public keys
/// - `deposits` - Table of Bitcoin deposits with UTXO references and amounts
/// - `assignments` - Table linking deposits to operators with execution deadlines
///
/// # Serialization
///
/// The state is serializable using Borsh for efficient storage and transmission
/// within the Anchor State Machine.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1State {
    /// Table of registered bridge operators.
    operators: OperatorTable,

    /// Table of Bitcoin deposits managed by the bridge.
    deposits: DepositsTable,

    /// Table of operator assignments for withdrawal processing.
    assignments: AssignmentTable,
}

impl BridgeV1State {
    /// Creates a new empty bridge state.
    ///
    /// Initializes all component tables as empty, ready for operator
    /// registrations and deposit processing.
    ///
    /// # Returns
    ///
    /// A new [`BridgeV1State`] instance with empty tables.
    pub fn new() -> Self {
        Self {
            operators: OperatorTable::new_empty(),
            deposits: DepositsTable::new_empty(),
            assignments: AssignmentTable::new_empty(),
        }
    }

    /// Returns a reference to the operator table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`OperatorTable`].
    pub fn operators(&self) -> &OperatorTable {
        &self.operators
    }

    /// Returns a mutable reference to the operator table.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`OperatorTable`].
    pub fn operators_mut(&mut self) -> &mut OperatorTable {
        &mut self.operators
    }

    /// Returns a reference to the deposits table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`DepositsTable`].
    pub fn deposits(&self) -> &DepositsTable {
        &self.deposits
    }

    /// Returns a mutable reference to the deposits table.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`DepositsTable`].
    pub fn deposits_mut(&mut self) -> &mut DepositsTable {
        &mut self.deposits
    }

    /// Returns a reference to the assignments table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`AssignmentTable`].
    pub fn assignments(&self) -> &AssignmentTable {
        &self.assignments
    }

    /// Returns a mutable reference to the assignments table.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`AssignmentTable`].
    pub fn assignments_mut(&mut self) -> &mut AssignmentTable {
        &mut self.assignments
    }
}

impl Default for BridgeV1State {
    /// Creates a default empty bridge state.
    ///
    /// Equivalent to [`BridgeV1State::new()`].
    fn default() -> Self {
        Self::new()
    }
}
