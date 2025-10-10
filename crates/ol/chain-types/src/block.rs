use strata_asm_common::AsmLogEntry;
use strata_primitives::buf::{Buf32, Buf64};

use crate::{Epoch, OLBlockId, OLTransaction, Slot};

/// The Orchestration Layer(OL) block.
#[derive(Clone, Debug)]
pub struct OLBlock {
    signed_header: SignedOLBlockHeader,
    body: OLBlockBody,
}

impl OLBlock {
    pub fn new(signed_header: SignedOLBlockHeader, body: OLBlockBody) -> Self {
        Self {
            signed_header,
            body,
        }
    }

    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }

    pub fn signed_header(&self) -> &SignedOLBlockHeader {
        &self.signed_header
    }

    pub fn header(&self) -> &OLBlockHeader {
        self.signed_header.header()
    }
}

/// OL Block header with signature.
#[derive(Clone, Debug)]
pub struct SignedOLBlockHeader {
    header: OLBlockHeader,
    signature: Buf64,
}

impl SignedOLBlockHeader {
    pub fn new(header: OLBlockHeader, signature: Buf64) -> Self {
        Self { header, signature }
    }

    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    pub fn signature(&self) -> Buf64 {
        self.signature
    }
}

/// OL Block header without signature.
#[derive(Clone, Debug)]
pub struct OLBlockHeader {
    /// The timestamp the block was created at.
    timestamp: u64,
    /// Slot the block was created for.
    slot: Slot,
    /// Epoch the block was created in.
    epoch: Epoch,
    /// Parent block id.
    parent_blk_id: OLBlockId,
    /// Root of the block body.
    body_root: Buf32,
    /// Root of the block logs.
    logs_root: Buf32,
    /// The state root resulting after the block execution.
    state_root: Buf32,
}

impl OLBlockHeader {
    pub fn new(
        timestamp: u64,
        slot: Slot,
        epoch: Epoch,
        parent_blk_id: OLBlockId,
        body_root: Buf32,
        logs_root: Buf32,
        state_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_blk_id,
            body_root,
            logs_root,
            state_root,
        }
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn parent_blk_id(&self) -> Buf32 {
        self.parent_blk_id
    }

    pub fn body_root(&self) -> Buf32 {
        self.body_root
    }

    pub fn logs_root(&self) -> Buf32 {
        self.logs_root
    }

    pub fn state_root(&self) -> Buf32 {
        self.state_root
    }
}

/// OL block body containing transactions and l1 updates
#[derive(Clone, Debug)]
pub struct OLBlockBody {
    /// The transactions contained in an OL block.
    txs: Vec<OLTransaction>,
    /// Updates from L1
    l1_update: L1Update,
}

impl OLBlockBody {
    pub fn new(txs: Vec<OLTransaction>, l1_update: L1Update) -> Self {
        Self { txs, l1_update }
    }

    pub fn txs(&self) -> &[OLTransaction] {
        &self.txs
    }

    pub fn l1_update(&self) -> &L1Update {
        &self.l1_update
    }
}

/// Represents an update from L1.
#[derive(Clone, Debug)]
pub struct L1Update {
    /// The state root before applying updates from L1.
    pub preseal_state_root: Buf32,

    /// L1 height the manifests are read upto.
    pub new_l1_blk_height: u64,

    /// L1 block hash the manifests are read upto.
    pub new_l1_blk_hash: Buf32,

    /// Manifests from last l1 height to the new l1 height.
    pub manifests: Vec<AsmManifest>,
}

impl L1Update {
    pub fn new(
        preseal_state_root: Buf32,
        new_l1_blk_height: u64,
        new_l1_blk_hash: Buf32,
        manifests: Vec<AsmManifest>,
    ) -> Self {
        Self {
            preseal_state_root,
            new_l1_blk_height,
            new_l1_blk_hash,
            manifests,
        }
    }

    pub fn preseal_state_root(&self) -> Buf32 {
        self.preseal_state_root
    }

    pub fn new_l1_blk_height(&self) -> u64 {
        self.new_l1_blk_height
    }

    pub fn new_l1_blk_hash(&self) -> Buf32 {
        self.new_l1_blk_hash
    }
}

/// A manifest containing ASM data corresponding to a L1 block.
#[derive(Debug, Clone)]
pub struct AsmManifest {
    /// L1 block id.
    blockid: Buf32,

    /// Logs from ASM STF.
    logs: Vec<AsmLogEntry>,
}

impl AsmManifest {
    pub fn new(blockid: Buf32, logs: Vec<AsmLogEntry>) -> Self {
        Self { blockid, logs }
    }

    pub fn blockid(&self) -> &Buf32 {
        &self.blockid
    }

    pub fn logs(&self) -> &[AsmLogEntry] {
        &self.logs
    }
}
