use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{
    actions::PendingUpgradeAction,
    crypto::{PubKey, Signature},
    error::UpgradeError,
    roles::Role,
    state::UpgradeSubprotoState,
    vote::AggregatedVote,
};

pub const SEQUENCER_UPDATE_TX_TYPE: u8 = 4;

pub const SEQUENCER_UPDATE_ENACTMENT_DELAY: u64 = 2016;

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SequencerUpdate {
    new_sequencer_pub_key: PubKey,
}

impl SequencerUpdate {
    pub fn new(new_sequencer_pub_key: PubKey) -> Self {
        Self {
            new_sequencer_pub_key,
        }
    }
}

pub fn handle_sequencer_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    // Extract sequencer update and vote
    let (update, vote) = extract_seq_update(tx)?;

    // StrataAdmin has the exclusive authority to update bridge operators
    let role = Role::StrataAdmin;

    // Fetch multisig configuration
    let config = state
        .get_multisig_authority_config(&role)
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    // Compute ActionId and validate the vote for the action
    let action = update.into();
    vote.validate_action(&config.keys, &action)?;

    // Create the pending action and enqueue it
    let pending_action = PendingUpgradeAction::new(action, SEQUENCER_UPDATE_ENACTMENT_DELAY, role);
    state.add_pending_action(pending_action);

    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_seq_update(tx: &TxInput<'_>) -> Result<(SequencerUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), SEQUENCER_UPDATE_TX_TYPE);

    let action = SequencerUpdate::new(PubKey::default());
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
