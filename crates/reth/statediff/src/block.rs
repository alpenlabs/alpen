use alloy_primitives::U256;
use revm::{database::BundleState, state::Bytecode};
use revm_primitives::{Address, HashMap, HashSet};
use serde::{Deserialize, Serialize};

use crate::account::{Account, AccountChanges};

/// Represents a full state diff for the block together with original values.
#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq)]
pub struct BlockStateDiff {
    /// Account state.
    pub state: HashMap<Address, AccountChanges>,
    /// All created contracts in this block.
    pub contracts: HashSet<Bytecode>,
}

/// A representation of the diff for several blocks [`BlockStateDiff`] ready to be posted on DA.
///
/// TODO: currently the representation is not based on "diffs", but it just sets new value
/// for the accounts and slots every time.
/// This will be changed once diff library is settled and ready.
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct BatchStateDiff {
    /// An account representation in the diff.
    /// [`Option::None`] indicates the account was destructed, but existed before the batch.
    pub accounts: HashMap<Address, Option<Account>>,

    /// A collection of deployed smart contracts within the batch.
    pub contracts: HashSet<Bytecode>,

    /// Storage slots for the account.
    pub storage_slots: HashMap<Address, HashMap<U256, U256>>,
}

impl From<BundleState> for BlockStateDiff {
    fn from(value: BundleState) -> Self {
        Self {
            state: value
                .state
                .into_iter()
                .map(|(k, v)| (k, AccountChanges::from(v)))
                .collect(),
            contracts: value.contracts.values().cloned().collect(),
        }
    }
}
