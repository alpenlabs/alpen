use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SequencerUpdate<T: BorshSerialize + BorshDeserialize> {
    new_sequencer_pub_key: T,
}
