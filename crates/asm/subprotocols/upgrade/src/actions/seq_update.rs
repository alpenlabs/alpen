use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{
    error::UpgradeError,
    state::UpgradeSubprotoState,
    crypto::{PubKey, Signature},
    vote::AggregatedVote,
};

pub const SEQUENCER_UPDATE_TX_TYPE: u8 = 4;

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
    relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_multisig_update(
    tx: &TxInput<'_>,
) -> Result<(SequencerUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), SEQUENCER_UPDATE_TX_TYPE);

    let action = SequencerUpdate::new(PubKey::default());
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
