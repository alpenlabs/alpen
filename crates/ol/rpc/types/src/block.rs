use serde::{Deserialize, Serialize};
use ssz::Encode;
use strata_identifiers::{Epoch, Slot};
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::{HexBytes, OLBlockId};

/// Rpc version of OL block entry in a slot range.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcBlockRangeEntry {
    slot: Slot,
    epoch: Epoch,
    blkid: OLBlockId,
    raw_block: HexBytes,
}

impl RpcBlockRangeEntry {
    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn blkid(&self) -> OLBlockId {
        self.blkid
    }

    pub fn raw_block(&self) -> &HexBytes {
        &self.raw_block
    }
}

impl From<&OLBlock> for RpcBlockRangeEntry {
    fn from(block: &OLBlock) -> Self {
        Self {
            slot: block.header().slot(),
            epoch: block.header().epoch(),
            blkid: block.header().compute_blkid(),
            raw_block: HexBytes(block.as_ssz_bytes()),
        }
    }
}
