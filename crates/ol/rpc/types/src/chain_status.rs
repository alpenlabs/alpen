use serde::{Deserialize, Serialize};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

/// OL chain status with tip block, parent epoch, confirmed epoch and finalized epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLChainStatus {
    /// Tip block commitment.
    pub tip: OLBlockCommitment,

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
        tip: OLBlockCommitment,
        parent: EpochCommitment,
        confirmed: EpochCommitment,
        finalized: EpochCommitment,
    ) -> Self {
        Self {
            tip,
            parent,
            confirmed,
            finalized,
        }
    }

    /// Returns the tip block commitment.
    pub fn tip(&self) -> &OLBlockCommitment {
        &self.tip
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
