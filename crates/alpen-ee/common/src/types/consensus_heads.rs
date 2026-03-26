use strata_acct_types::Hash;
use strata_identifiers::Epoch;

/// Consensus block hashes and epoch numbers for confirmed and finalized EE state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusHeads {
    /// Confirmed EE block hash.
    pub confirmed: Hash,
    /// OL epoch of the confirmed frontier.
    pub confirmed_epoch: Epoch,
    /// Finalized EE block hash.
    pub finalized: Hash,
    /// OL epoch of the finalized frontier.
    pub finalized_epoch: Epoch,
}

impl ConsensusHeads {
    /// Returns the confirmed block hash.
    pub fn confirmed(&self) -> &Hash {
        &self.confirmed
    }

    /// Returns the OL epoch of the confirmed frontier.
    pub fn confirmed_epoch(&self) -> Epoch {
        self.confirmed_epoch
    }

    /// Returns the finalized block hash.
    pub fn finalized(&self) -> &Hash {
        &self.finalized
    }

    /// Returns the OL epoch of the finalized frontier.
    pub fn finalized_epoch(&self) -> Epoch {
        self.finalized_epoch
    }
}
