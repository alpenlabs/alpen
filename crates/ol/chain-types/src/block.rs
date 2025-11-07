use strata_asm_common::AsmLogEntry;
use strata_primitives::{
    Epoch, Slot,
    buf::{Buf32, Buf64},
};

use crate::{OLBlockId, OLTransaction};

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
    parent_blkid: OLBlockId,

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
        parent_blkid: OLBlockId,
        body_root: Buf32,
        logs_root: Buf32,
        state_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_blkid,
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

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn parent_blkid(&self) -> Buf32 {
        self.parent_blkid
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

    pub fn compute_root(&self) -> Buf32 {
        // TODO: use ssz
        todo!()
    }
}

/// OL block body containing transactions and l1 updates
#[derive(Clone, Debug)]
pub struct OLBlockBody {
    /// The transactions contained in an OL block.
    tx_segment: OLTxSegment,

    /// Updates from L1.
    l1_update: Option<L1Update>,
}

impl OLBlockBody {
    pub fn new(tx_segment: OLTxSegment, l1_update: Option<L1Update>) -> Self {
        Self {
            tx_segment,
            l1_update,
        }
    }

    pub fn txs(&self) -> &[OLTransaction] {
        self.tx_segment.txs()
    }

    pub fn l1_update(&self) -> &Option<L1Update> {
        &self.l1_update
    }

    pub fn tx_segment(&self) -> &OLTxSegment {
        &self.tx_segment
    }

    pub fn compute_root(&self) -> Buf32 {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub struct OLTxSegment {
    /// Transactions in the segment.
    txs: Vec<OLTransaction>,
    // Add other attributes.
}

impl OLTxSegment {
    pub fn new(txs: Vec<OLTransaction>) -> Self {
        Self { txs }
    }

    pub fn txs(&self) -> &[OLTransaction] {
        &self.txs
    }
}

/// Represents an update from L1.
#[derive(Clone, Debug)]
pub struct L1Update {
    /// The state root before applying updates from L1.
    pub preseal_state_root: Buf32,

    /// Manifests from last l1 height to the new l1 height.
    pub manifests: Vec<AsmManifest>,
}

impl L1Update {
    pub fn new(preseal_state_root: Buf32, manifests: Vec<AsmManifest>) -> Self {
        Self {
            preseal_state_root,
            manifests,
        }
    }

    pub fn preseal_state_root(&self) -> Buf32 {
        self.preseal_state_root
    }

    pub fn manifests(&self) -> &[AsmManifest] {
        &self.manifests
    }
}

/// A manifest containing ASM data corresponding to a L1 block.
#[derive(Debug, Clone)]
pub struct AsmManifest {
    /// L1 block id.
    l1_blkid: Buf32,

    /// Logs from ASM STF.
    logs: Vec<AsmLogEntry>,
}

impl AsmManifest {
    pub fn new(l1_blkid: Buf32, logs: Vec<AsmLogEntry>) -> Self {
        Self { l1_blkid, logs }
    }

    pub fn l1_blkid(&self) -> Buf32 {
        self.l1_blkid
    }

    pub fn logs(&self) -> &[AsmLogEntry] {
        &self.logs
    }
}
