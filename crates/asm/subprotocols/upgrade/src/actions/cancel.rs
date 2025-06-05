use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};
use strata_primitives::buf::Buf32;

use super::ActionId;
use crate::{
    crypto::Signature, error::UpgradeError, state::UpgradeSubprotoState, vote::AggregatedVote,
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
    _relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    let (update, vote) = extract_cancel_action(tx)?;

    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_cancel_action(tx: &TxInput<'_>) -> Result<(CancelAction, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), CANCEL_TX_TYPE);

    let id = Buf32::zero().into();
    let action = CancelAction::new(id);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());
    Ok((action, vote))
}
