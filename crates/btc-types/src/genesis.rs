use serde::{Deserialize, Serialize};
use strata_identifiers::{L1BlockCommitment, L1BlockId};

pub const TIMESTAMPS_FOR_MEDIAN: usize = 11;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GenesisL1View {
    pub blk: L1BlockCommitment,
    pub next_target: u32,
    pub epoch_start_timestamp: u32,
    pub last_11_timestamps: [u32; TIMESTAMPS_FOR_MEDIAN],
}

impl GenesisL1View {
    pub fn height(&self) -> u32 {
        self.blk.height()
    }

    pub fn height_u64(&self) -> u64 {
        self.blk.height_u64()
    }

    pub fn blkid(&self) -> L1BlockId {
        *self.blk.blkid()
    }
}
