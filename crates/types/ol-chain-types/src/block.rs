use sha2::{Digest, Sha256};
use strata_asm_common::AsmLogEntry;
use strata_primitives::buf::{Buf32, Buf64};

use crate::transaction::{OLTransaction, TransactionPayload};

type OLBlockId = Buf32; // TODO: change this later
pub type Slot = u64;
type Epoch = u64;

/// Represents a complete block in the Orchestration Layer (OL) chain
#[derive(Debug, Clone)]
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

    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }
}
/// A block header with a cryptographic signature
#[derive(Debug, Clone)]
pub struct SignedOLBlockHeader {
    header: OLBlockHeader,
    signature: Buf64,
}

/// The header portion of an OL block containing metadata
#[derive(Debug, Clone)]
pub struct OLBlockHeader {
    /// The timestamp this block was constructed at.
    timestamp: u64,

    /// Slot this block is constructed for.
    slot: Slot,

    /// Epoch this block belongs to.
    epoch: Epoch,

    /// Parent block id.
    parent_blockid: OLBlockId,

    /// Root of the logs that are generated after block execution.
    logs_root: Buf32,

    /// Root that commits to the content of an `OLBlock`.
    body_root: Buf32,

    /// Root of the state this block moves to.
    state_root: Buf32,
}

impl OLBlockHeader {
    pub fn new(
        timestamp: u64,
        slot: Slot,
        epoch: Epoch,
        parent_blockid: OLBlockId,
        logs_root: Buf32,
        body_root: Buf32,
        state_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_blockid,
            logs_root,
            body_root,
            state_root,
        }
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn parent_blockid(&self) -> &OLBlockId {
        &self.parent_blockid
    }

    pub fn logs_root(&self) -> &Buf32 {
        &self.logs_root
    }

    pub fn body_root(&self) -> &Buf32 {
        &self.body_root
    }

    pub fn state_root(&self) -> &Buf32 {
        &self.state_root
    }

    // NOTE: this will possibly be redundant once we have SSZ
    pub fn compute_header_root(&self) -> Buf32 {
        let mut hasher = Sha256::new();

        // Hash all header fields in a deterministic order
        hasher.update(self.timestamp.to_be_bytes());
        hasher.update(self.slot.to_be_bytes());
        hasher.update(self.epoch.to_be_bytes());
        hasher.update(self.parent_blockid.as_ref());
        hasher.update(self.body_root.as_ref());
        hasher.update(self.state_root.as_ref());

        Buf32::new(hasher.finalize().into())
    }
}

/// The body portion of an OL block containing the actual data
#[derive(Debug, Clone)]
pub struct OLBlockBody {
    txs: Option<Vec<OLTransaction>>,
    l1update: Option<L1Update>,
}

impl OLBlockBody {
    pub fn new(txs: Option<Vec<OLTransaction>>, l1update: Option<L1Update>) -> Self {
        Self { txs, l1update }
    }

    pub fn txs(&self) -> &Option<Vec<OLTransaction>> {
        &self.txs
    }

    pub fn l1update(&self) -> &Option<L1Update> {
        &self.l1update
    }

    // NOTE: this will be redundant after ssz
    pub fn compute_root(&self) -> Buf32 {
        let mut hasher = Sha256::new();
        if let Some(txs) = self.txs() {
            for tx in txs {
                match tx.payload() {
                    TransactionPayload::GenericAccountMessage { target: _, payload } => {
                        // hasher.update(target.as_slice());
                        hasher.update(payload);
                    }
                    TransactionPayload::SnarkAccountUpdate {
                        target: _,
                        update: _,
                    } => {
                        // hasher.update(target.as_slice());
                        // hasher.update(&update.update_proof);
                        // hasher.update(update.data.seq_no.to_be_bytes());
                        // TODO: other fields, maybe wait for ssz?
                        todo!()
                    }
                }
            }
        }
        Buf32::new(hasher.finalize().into())
    }
}
/// Represents an update from Layer 1 blockchain
#[derive(Debug, Clone)]
pub struct L1Update {
    /// The state root before applying updates from L1
    pub preseal_state_root: Buf32,

    /// L1 height the manifests are read upto
    pub new_l1_blk_height: u64,

    /// L1 block hash the manifests are read upto
    pub new_l1_blk_hash: Buf32,

    /// Manifests from last l1_height to the new_l1_height
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

    pub fn preseal_state_root(&self) -> &Buf32 {
        &self.preseal_state_root
    }

    pub fn new_l1_blk_height(&self) -> u64 {
        self.new_l1_blk_height
    }

    pub fn new_l1_blk_hash(&self) -> Buf32 {
        self.new_l1_blk_hash
    }

    pub fn manifests(&self) -> &[AsmManifest] {
        &self.manifests
    }
}

/// A manifest containing ASM data
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

impl SignedOLBlockHeader {
    pub fn new(header: OLBlockHeader, signature: Buf64) -> Self {
        Self { header, signature }
    }

    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }
}
