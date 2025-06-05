use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};
use zkaleido::VerifyingKey;

use crate::{
    error::UpgradeError, roles::StrataProof, state::UpgradeSubprotoState, crypto::Signature,
    vote::AggregatedVote,
};

pub const VK_UPDATE_TX_TYPE: u8 = 2;

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
}

pub fn handle_vk_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_multisig_update(
    tx: &TxInput<'_>,
) -> Result<(VerifyingKeyUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), VK_UPDATE_TX_TYPE);

    let action = VerifyingKeyUpdate::new(VerifyingKey::default(), StrataProof::OlStf);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
