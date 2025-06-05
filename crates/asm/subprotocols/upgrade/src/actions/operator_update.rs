use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{
    error::UpgradeError,
    state::UpgradeSubprotoState,
    types::{PubKey, Signature},
    vote::AggregatedVote,
};

pub const OPERATOR_UPDATE_TX_TYPE: u8 = 3;

/// Represents a change to the Bridge Operator Set`
/// * removes the specified `old_members` from the set
/// * adds the specified `new_members`
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OperatorSetUpdate {
    new_members: Vec<PubKey>,
    old_members: Vec<PubKey>,
}

impl OperatorSetUpdate {
    pub fn new(new_members: Vec<PubKey>, old_members: Vec<PubKey>) -> Self {
        Self {
            new_members,
            old_members,
        }
    }
}

pub fn handle_operator_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_multisig_update(
    tx: &TxInput<'_>,
) -> Result<(OperatorSetUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), OPERATOR_UPDATE_TX_TYPE);

    let action = OperatorSetUpdate::new(vec![], vec![]);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
