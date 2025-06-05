use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use super::ActionId;
use crate::{
    error::UpgradeError, state::UpgradeSubprotoState, types::Signature, vote::AggregatedVote,
};

pub const CANCEL_TX_TYPE: u8 = 5;

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, BorshSerialize, BorshDeserialize)]
pub struct CancelAction {
    id: ActionId,
}

impl CancelAction {
    pub fn new(id: ActionId) -> Self {
        CancelAction { id }
    }
}

pub fn handle_cancel_action(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_cancel_action(tx: &TxInput<'_>) -> Result<(CancelAction, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), CANCEL_TX_TYPE);

    let id = ActionId([0u8; 32]);
    let action = CancelAction::new(id);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());
    Ok((action, vote))
}
