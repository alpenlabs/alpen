use strata_asm_common::AsmLogEntry;
use strata_identifiers::{Buf32, Buf64};

use crate::{Epoch, OLBlockId, OLTransaction, Slot};

/// Signed full orchestration layer block.
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

    pub fn signed_header(&self) -> &SignedOLBlockHeader {
        &self.signed_header
    }

    /// Returns the actual block header inside the signed header structure.
    pub fn header(&self) -> &OLBlockHeader {
        self.signed_header.header()
    }

    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }
}

/// OL header with signature.
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

    /// This MUST be a schnorr signature for now.
    pub fn signature(&self) -> Buf64 {
        self.signature
    }
}

/// OL header.
///
/// This should not be directly used itself during execution.
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

    /// The state root resulting after the block execution.
    state_root: Buf32,

    /// Root of the block logs.
    logs_root: Buf32,
}

impl OLBlockHeader {
    pub fn new(
        timestamp: u64,
        slot: Slot,
        epoch: Epoch,
        parent_blkid: OLBlockId,
        body_root: Buf32,
        state_root: Buf32,
        logs_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_blkid,
            body_root,
            state_root,
            logs_root,
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

    pub fn parent_blkid(&self) -> Buf32 {
        self.parent_blkid
    }

    pub fn body_root(&self) -> Buf32 {
        self.body_root
    }

    pub fn state_root(&self) -> Buf32 {
        self.state_root
    }

    pub fn logs_root(&self) -> Buf32 {
        self.logs_root
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
    pub(crate) fn new(tx_segment: OLTxSegment, l1_update: Option<L1Update>) -> Self {
        Self {
            tx_segment,
            l1_update,
        }
    }

    pub fn new_regular(tx_segment: OLTxSegment) -> Self {
        Self::new(tx_segment, None)
    }

    // TODO convert to builder?
    pub fn set_l1_update(&mut self, l1_update: L1Update) {
        self.l1_update = Some(l1_update);
    }

    pub fn tx_segment(&self) -> &OLTxSegment {
        &self.tx_segment
    }

    pub fn l1_update(&self) -> Option<&L1Update> {
        self.l1_update.as_ref()
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

    /// If there are new manifests (which there should be), returns the blkid of
    /// the last one.  This is the blkid that we use as the new L1 chain tip.
    pub fn new_l1_blkid(&self) -> Option<&Buf32> {
        self.manifests.last().map(|mf| mf.l1blkid())
    }
}

/// A manifest containing ASM data corresponding to a L1 block.
// TODO maybe convert to using exported types from ASM crates?
#[derive(Debug, Clone)]
pub struct AsmManifest {
    /// L1 block id.
    l1blkid: Buf32,

    /// Logs from ASM STF.
    logs: Vec<AsmLogEntry>,
    // TODO add wtxs root
}

impl AsmManifest {
    pub fn new(l1blkid: Buf32, logs: Vec<AsmLogEntry>) -> Self {
        Self { l1blkid, logs }
    }

    pub fn l1blkid(&self) -> &Buf32 {
        &self.l1blkid
    }

    pub fn logs(&self) -> &[AsmLogEntry] {
        &self.logs
    }
}
