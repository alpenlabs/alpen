use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{
    error::UpgradeError,
    roles::Role,
    state::UpgradeSubprotoState,
    types::{PubKey, Signature},
    vote::AggregatedVote,
};

pub const MULTISIG_CONFIG_UPDATE_TX_TYPE: u8 = 1;

/// Represents a change to the multisig configuration for the given `role`:
/// * removes the specified `old_members` from the set,
/// * adds the specified `new_members`
/// * updates the threshold.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfigUpdate {
    new_members: Vec<PubKey>,
    old_members: Vec<PubKey>,
    new_threshold: u8,
    role: Role,
}

impl MultisigConfigUpdate {
    pub fn new(
        new_members: Vec<PubKey>,
        old_members: Vec<PubKey>,
        new_threshold: u8,
        role: Role,
    ) -> Self {
        Self {
            new_members,
            old_members,
            new_threshold,
            role,
        }
    }
}

pub fn handle_multisig_config_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_multisig_update(
    tx: &TxInput<'_>,
) -> Result<(MultisigConfigUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), MULTISIG_CONFIG_UPDATE_TX_TYPE);

    let action = MultisigConfigUpdate::new(vec![], vec![], 0, Role::BridgeAdmin);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
