use serde::{Deserialize, Serialize};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

/// OL chain status with tip block, latest summarized epoch, confirmed epoch, and finalized epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLChainStatus {
    /// Tip block commitment.
    pub tip: OLBlockCommitment,

    /// Latest summarized epoch commitment.
    pub latest: EpochCommitment,

    /// Confirmed epoch commitment.
    pub confirmed: EpochCommitment,

    /// Finalized epoch commitment.
    pub finalized: EpochCommitment,
}

impl RpcOLChainStatus {
    /// Creates a new [`RpcOLChainStatus`].
    pub fn new(
        tip: OLBlockCommitment,
        latest: EpochCommitment,
        confirmed: EpochCommitment,
        finalized: EpochCommitment,
    ) -> Self {
        Self {
            tip,
            latest,
            confirmed,
            finalized,
        }
    }

    /// Returns the tip block commitment.
    pub fn tip(&self) -> &OLBlockCommitment {
        &self.tip
    }

    /// Returns the latest summarized epoch commitment.
    pub fn latest(&self) -> &EpochCommitment {
        &self.latest
    }

    /// Returns the confirmed epoch commitment.
    pub fn confirmed(&self) -> &EpochCommitment {
        &self.confirmed
    }

    /// Returns the finalized epoch commitment.
    pub fn finalized(&self) -> &EpochCommitment {
        &self.finalized
    }
}
