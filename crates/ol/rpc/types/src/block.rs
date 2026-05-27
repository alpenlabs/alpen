use serde::{Deserialize, Serialize};
use ssz::Encode;
use strata_identifiers::{Epoch, Slot};
use strata_ol_chain_types_new::{OLAsmManifestContainer, OLBlock};
use strata_primitives::{HexBytes, HexBytes32, HexBytes64, OLBlockId};

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

/// Lightweight summary of an OL block for list views.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLBlockSummary {
    slot: Slot,
    epoch: Epoch,
    blkid: OLBlockId,
    timestamp: u64,
    tx_count: u32,
    is_terminal: bool,
}

impl RpcOLBlockSummary {
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

    pub fn tx_count(&self) -> u32 {
        self.tx_count
    }

    pub fn is_terminal(&self) -> bool {
        self.is_terminal
    }
}

impl From<&OLBlock> for RpcOLBlockSummary {
    fn from(block: &OLBlock) -> Self {
        let header = block.header();
        let tx_count = block
            .body()
            .tx_segment()
            .map(|seg| seg.txs().len() as u32)
            .unwrap_or(0);
        Self {
            slot: header.slot(),
            epoch: header.epoch(),
            blkid: header.compute_blkid(),
            timestamp: header.timestamp(),
            tx_count,
            is_terminal: header.is_terminal(),
        }
    }
}

/// Decoded summary of an OL block returned by `getBlockBySlot`.
///
/// Composes [`RpcBlockHeaderEntry`] with selected credential and body fields
/// decoded from the SSZ block.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLBlockDetail {
    /// Decoded header fields.
    header: RpcBlockHeaderEntry,
    /// Schnorr signature over the header, if present.
    signature: Option<HexBytes64>,
    /// Number of transactions in the block.
    tx_count: u32,
    /// Manifest summary, present on blocks carrying ASM manifests (which may
    /// appear in any block, not only terminal ones).
    manifests: Option<RpcOLManifestsSummary>,
}

impl RpcOLBlockDetail {
    pub fn header(&self) -> &RpcBlockHeaderEntry {
        &self.header
    }

    pub fn signature(&self) -> Option<&HexBytes64> {
        self.signature.as_ref()
    }

    pub fn tx_count(&self) -> u32 {
        self.tx_count
    }

    pub fn manifests(&self) -> Option<&RpcOLManifestsSummary> {
        self.manifests.as_ref()
    }
}

impl From<&OLBlock> for RpcOLBlockDetail {
    fn from(block: &OLBlock) -> Self {
        let header = RpcBlockHeaderEntry::from(block);
        let signature = block
            .signed_header()
            .signature()
            .map(|sig| HexBytes64::from(sig.0));
        let body = block.body();
        let tx_count = body
            .tx_segment()
            .map(|seg| seg.txs().len() as u32)
            .unwrap_or(0);
        let manifests = body.manifests().map(RpcOLManifestsSummary::from);
        Self {
            header,
            signature,
            tx_count,
            manifests,
        }
    }
}

/// Summary of the ASM manifests carried by an OL block.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLManifestsSummary {
    /// Number of ASM manifests included in the block.
    manifest_count: u32,
}

impl RpcOLManifestsSummary {
    pub fn manifest_count(&self) -> u32 {
        self.manifest_count
    }
}

impl From<&OLAsmManifestContainer> for RpcOLManifestsSummary {
    fn from(container: &OLAsmManifestContainer) -> Self {
        Self {
            manifest_count: container.manifests().len() as u32,
        }
    }
}
