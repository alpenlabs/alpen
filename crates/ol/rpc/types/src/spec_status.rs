//! OL spec status at the canonical tip.

use serde::{Deserialize, Serialize};
use strata_identifiers::L1Height;

use crate::RpcOLBlockInfo;

/// The OL spec the canonical tip runs under, and the spec of the next epoch
/// once the tip ends its epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLSpecStatus {
    /// Tip block info.
    pub tip: RpcOLBlockInfo,

    /// Raw version of the spec the tip's epoch runs under.
    pub cur_spec_version: u32,

    /// Raw version of the spec the epoch after the tip's runs under.
    ///
    /// `None` while the tip's epoch is open: whether it processes a
    /// checkpoint predicate enactment is not known until it ends.
    pub next_epoch_spec_version: Option<u32>,

    /// Set when the next epoch runs a spec this binary does not implement.
    /// The node builds, executes and applies nothing past the tip until it is
    /// upgraded.
    pub upgrade_required: Option<RpcUpgradeRequired>,
}

impl RpcOLSpecStatus {
    /// Creates a new [`RpcOLSpecStatus`].
    pub fn new(
        tip: RpcOLBlockInfo,
        cur_spec_version: u32,
        next_epoch_spec_version: Option<u32>,
        upgrade_required: Option<RpcUpgradeRequired>,
    ) -> Self {
        Self {
            tip,
            cur_spec_version,
            next_epoch_spec_version,
            upgrade_required,
        }
    }

    /// Returns the tip block info.
    pub fn tip(&self) -> &RpcOLBlockInfo {
        &self.tip
    }

    /// Returns the raw version of the spec the tip's epoch runs under.
    pub fn cur_spec_version(&self) -> u32 {
        self.cur_spec_version
    }

    /// Returns the raw version of the spec the epoch after the tip's runs
    /// under, if the tip ends its epoch.
    pub fn next_epoch_spec_version(&self) -> Option<u32> {
        self.next_epoch_spec_version
    }

    /// Returns the upgrade the next epoch needs, if any.
    pub fn upgrade_required(&self) -> Option<&RpcUpgradeRequired> {
        self.upgrade_required.as_ref()
    }
}

/// A spec the next epoch runs that this binary does not implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcUpgradeRequired {
    /// L1 height of the checkpoint predicate enactment that ends the current
    /// spec.
    pub enactment_l1_height: L1Height,

    /// Raw version of the spec the enactment activates.
    pub spec_version: u32,
}

impl RpcUpgradeRequired {
    /// Creates a new [`RpcUpgradeRequired`].
    pub fn new(enactment_l1_height: L1Height, spec_version: u32) -> Self {
        Self {
            enactment_l1_height,
            spec_version,
        }
    }

    /// Returns the L1 height of the enactment that ends the current spec.
    pub fn enactment_l1_height(&self) -> L1Height {
        self.enactment_l1_height
    }

    /// Returns the raw version of the spec the enactment activates.
    pub fn spec_version(&self) -> u32 {
        self.spec_version
    }
}
