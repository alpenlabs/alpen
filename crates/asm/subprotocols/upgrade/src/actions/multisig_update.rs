use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{error::UpgradeError, roles::Role, state::UpgradeSubprotoState};
/// Represents a change to the multisig configuration for the given `role`:
/// * removes the specified `old_members` from the set,
/// * adds the specified `new_members`
/// * updates the threshold.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfigUpdate<T: BorshSerialize + BorshDeserialize> {
    new_members: Vec<T>,
    old_members: Vec<T>,
    new_threshold: u8,
    role: Role,
}

pub fn handle_multisig_config_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}
