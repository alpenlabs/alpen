use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};
use zkaleido::VerifyingKey;

use crate::{
    actions::PendingUpgradeAction,
    crypto::Signature,
    error::UpgradeError,
    multisig::vote::AggregatedVote,
    roles::{Role, StrataProof},
    state::UpgradeSubprotoState,
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
    pub fn new(new_vk: VerifyingKey, kind: StrataProof) -> Self {
        Self { new_vk, kind }
    }

    pub fn proof_kind(&self) -> &StrataProof {
        &self.kind
    }
}

pub fn handle_vk_update_tx(
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
        .get_multisig_config(&role)
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    // Validate the vote for the action
    let action = update.into();
    vote.validate_action(&config.keys, &action)?;

    // Create the pending action and enqueue it
    let pending_action = PendingUpgradeAction::new(action, delay, role);
    state.add_pending_action(pending_action);
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
