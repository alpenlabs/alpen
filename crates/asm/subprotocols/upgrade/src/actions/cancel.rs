use borsh::{BorshDeserialize, BorshSerialize};

use super::ActionId;

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, BorshSerialize, BorshDeserialize)]
pub struct CancelAction {
    id: ActionId,
}
