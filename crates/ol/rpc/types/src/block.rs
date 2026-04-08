use serde::{Deserialize, Serialize};
use ssz::Encode;
use strata_identifiers::{Epoch, Slot};
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::{HexBytes, HexBytes32, OLBlockId};

/// Rpc version of OL block entry in a slot range.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcBlockEntry {
    slot: Slot,
    epoch: Epoch,
    blkid: OLBlockId,
    raw_block: HexBytes,
}

impl RpcBlockEntry {
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

impl From<&OLBlock> for RpcBlockEntry {
    fn from(block: &OLBlock) -> Self {
        Self {
            slot: block.header().slot(),
            epoch: block.header().epoch(),
            blkid: block.header().compute_blkid(),
            raw_block: HexBytes(block.as_ssz_bytes()),
        }
    }
}

/// RPC version of an OL block header entry in a slot range.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcBlockHeaderEntry {
    slot: Slot,
    epoch: Epoch,
    blkid: OLBlockId,
    timestamp: u64,
    parent_blkid: OLBlockId,
    state_root: HexBytes32,
    body_root: HexBytes32,
    logs_root: HexBytes32,
    is_terminal: bool,
}

impl RpcBlockHeaderEntry {
    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn blkid(&self) -> OLBlockId {
        self.blkid
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn parent_blkid(&self) -> OLBlockId {
        self.parent_blkid
    }

    pub fn state_root(&self) -> &HexBytes32 {
        &self.state_root
    }

    pub fn body_root(&self) -> &HexBytes32 {
        &self.body_root
    }

    pub fn logs_root(&self) -> &HexBytes32 {
        &self.logs_root
    }

    pub fn is_terminal(&self) -> bool {
        self.is_terminal
    }
}

impl From<&OLBlock> for RpcBlockHeaderEntry {
    fn from(block: &OLBlock) -> Self {
        let header = block.header();
        Self {
            slot: header.slot(),
            epoch: header.epoch(),
            blkid: header.compute_blkid(),
            timestamp: header.timestamp(),
            parent_blkid: *header.parent_blkid(),
            state_root: HexBytes32::from(header.state_root().0),
            body_root: HexBytes32::from(header.body_root().0),
            logs_root: HexBytes32::from(header.logs_root().0),
            is_terminal: header.is_terminal(),
        }
    }
}
