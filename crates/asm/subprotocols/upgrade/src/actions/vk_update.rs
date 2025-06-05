use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};
use zkaleido::VerifyingKey;

use crate::{
    actions::{ActionId, PendingUpgradeAction},
    crypto::{Signature, tagged_hash},
    error::UpgradeError,
    roles::{Role, StrataProof},
    state::UpgradeSubprotoState,
    vote::AggregatedVote,
};

pub const VK_UPDATE_TX_TYPE: u8 = 2;

const ASM_VK_ENACTMENT_DELAY: u64 = 12_960;
const OL_STF_VK_ENACTMENT_DELAY: u64 = 4_320;

/// Represents an update to the verifying key used for a particular Strata
/// proof layer.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct VerifyingKeyUpdate {
    new_vk: VerifyingKey,
    kind: StrataProof,
}

impl VerifyingKeyUpdate {
    fn new(new_vk: VerifyingKey, kind: StrataProof) -> Self {
        Self { new_vk, kind }
    }

    pub fn compute_action_id(&self) -> ActionId {
        const PREFIX: &[u8] = b"VK_UPDATE";
        tagged_hash(PREFIX, self).into()
    }
}

pub fn handle_vk_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    // Extract VK update and vote
    let (update, vote) = extract_vk_update(tx)?;

    // Select the right role
    let (role, delay) = match update.kind {
        StrataProof::ASM => (Role::BridgeConsensusManager, ASM_VK_ENACTMENT_DELAY),
        StrataProof::OlStf => (Role::StrataConsensusManager, OL_STF_VK_ENACTMENT_DELAY),
    };

    // Fetch multisig configuration
    let config = state
        .get_multisig_authority_config(&role)
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    // Compute ActionId and validate the vote for the action
    let update_action_id = update.compute_action_id();
    vote.validate_action(&config.keys, &update_action_id)?;

    // Create the pending action and enqueue it
    let pending_action = PendingUpgradeAction::new(update.into(), delay, role);
    state.add_pending_action(update_action_id, pending_action);
    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_vk_update(
    tx: &TxInput<'_>,
) -> Result<(VerifyingKeyUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), VK_UPDATE_TX_TYPE);

    let action = VerifyingKeyUpdate::new(VerifyingKey::default(), StrataProof::OlStf);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
