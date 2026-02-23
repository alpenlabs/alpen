use serde::{Deserialize, Serialize};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

/// OL chain status with latest, confirmed, and finalized blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLChainStatus {
    /// Latest block commitment.
    pub latest: OLBlockCommitment,
    /// Confirmed block commitment.
    pub confirmed: EpochCommitment,
    /// Finalized block commitment.
    pub finalized: EpochCommitment,
}

impl RpcOLChainStatus {
    /// Creates a new [`RpcOLChainStatus`].
    pub fn new(
        latest: OLBlockCommitment,
        confirmed: EpochCommitment,
        finalized: EpochCommitment,
    ) -> Self {
        Self {
            latest,
            confirmed,
            finalized,
        }
    }

    /// Returns the latest block commitment.
    pub fn latest(&self) -> &OLBlockCommitment {
        &self.latest
    }

    /// Returns the confirmed block commitment.
    pub fn confirmed(&self) -> &EpochCommitment {
        &self.confirmed
    }

    /// Returns the finalized block commitment.
    pub fn finalized(&self) -> &EpochCommitment {
        &self.finalized
    }
}
