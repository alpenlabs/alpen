use futures::Stream;

use crate::account_state::AccountStateCommitment;

/// Consensus Update from OL/L1
#[derive(Debug, Clone)]
pub struct OLConsensusUpdate {
    confirmed: AccountStateCommitment,
    finalized: AccountStateCommitment,
}

impl OLConsensusUpdate {
    pub fn new(confirmed: AccountStateCommitment, finalized: AccountStateCommitment) -> Self {
        Self {
            confirmed,
            finalized,
        }
    }

    pub fn confirmed(&self) -> &AccountStateCommitment {
        &self.confirmed
    }

    pub fn finalized(&self) -> &AccountStateCommitment {
        &self.finalized
    }
}

pub trait OLConsensusTracker {
    fn subscribe(&self) -> impl Stream<Item = OLConsensusUpdate>;
}

/// Consensus update for blocks not yet checkpointed/proven on OL/L1.
/// Represents new blocks produced and signed by sequencer.
/// TODO: rename
#[derive(Debug, Clone)]
pub struct PreConsensusUpdate {
    preconfirmed: AccountStateCommitment,
}

impl PreConsensusUpdate {
    pub fn new(preconfirmed: AccountStateCommitment) -> Self {
        Self { preconfirmed }
    }

    pub fn preconfirmed(&self) -> &AccountStateCommitment {
        &self.preconfirmed
    }
}

pub trait PreConsensusTracker {
    fn subscribe(&self) -> impl Stream<Item = PreConsensusUpdate>;
}
