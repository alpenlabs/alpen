use bitcoin::Network;
use serde::{Deserialize, Serialize};

use crate::{rollup::RollupParams, sync::SyncParams};

/// Combined set of parameters across all the consensus logic.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Params {
    pub rollup: RollupParams,
    pub run: SyncParams,
}

impl Params {
    pub fn rollup(&self) -> &RollupParams {
        &self.rollup
    }

    pub fn run(&self) -> &SyncParams {
        &self.run
    }

    pub fn network(&self) -> Network {
        self.rollup.network
    }
}
