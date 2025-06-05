use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{error::UpgradeError, state::UpgradeSubprotoState};

/// Represents a change to the Bridge Operator Set`
/// * removes the specified `old_members` from the set
/// * adds the specified `new_members`
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OperatorSetUpdate<T: BorshSerialize + BorshDeserialize> {
    new_members: Vec<T>,
    old_members: Vec<T>,
}

pub fn handle_operator_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}
