use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use super::ActionId;
use crate::{error::UpgradeError, state::UpgradeSubprotoState};

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, BorshSerialize, BorshDeserialize)]
pub struct CancelAction {
    id: ActionId,
}

pub fn handle_cancel_action(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}

