use serde::{Deserialize, Serialize};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

/// OL chain status with latest block, parent epoch, confirmed epoch and finalized epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLChainStatus {
    /// Latest block commitment.
    pub latest: OLBlockCommitment,
    /// Parent epoch commitment (the most recently completed epoch).
    pub parent: EpochCommitment,
    /// Confirmed epoch commitment.
    ///
    /// Currently set to the same value as `parent` for compatibility.
    pub confirmed: EpochCommitment,
    /// Finalized epoch commitment.
    pub finalized: EpochCommitment,
}

impl RpcOLChainStatus {
    /// Creates a new [`RpcOLChainStatus`].
    pub fn new(
        latest: OLBlockCommitment,
        parent: EpochCommitment,
        confirmed: EpochCommitment,
        finalized: EpochCommitment,
    ) -> Self {
        Self {
            latest,
            parent,
            confirmed,
            finalized,
        }
    }

    /// Returns the latest block commitment.
    pub fn latest(&self) -> &OLBlockCommitment {
        &self.latest
    }

    /// Returns the parent epoch commitment.
    pub fn parent(&self) -> &EpochCommitment {
        &self.parent
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
