use borsh::{BorshDeserialize, BorshSerialize};

/// Represents a change to the Bridge Operator Set`
/// * removes the specified `old_members` from the set
/// * adds the specified `new_members`
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OperatorSetUpdate<T: BorshSerialize + BorshDeserialize> {
    new_members: Vec<T>,
    old_members: Vec<T>,
}
