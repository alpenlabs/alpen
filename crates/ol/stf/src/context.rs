//! Context types for tracking state across validation.

use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::{Epoch, OLBlockHeader, OLLog, Slot};

/// Block info context.
///
/// This contains some information that would normally be in the header but that
/// we can know in advance of executing the block.
#[derive(Clone, Debug)]
pub struct BlockContext {
    timestamp: u64,
    slot: Slot,
    epoch: Epoch,
    parent_header: Option<OLBlockHeader>,
}

impl BlockContext {
    pub(crate) fn new(
        timestamp: u64,
        slot: Slot,
        epoch: Epoch,
        parent_header: Option<OLBlockHeader>,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_header,
        }
    }

    /// Constructs a context for regular blocks from their headers.
    pub fn from_headers(bh: &OLBlockHeader, parent: OLBlockHeader) -> Self {
        Self::new(bh.timestamp(), bh.slot(), bh.epoch(), Some(parent))
    }

    /// Constructs a context for the genesis block.
    pub fn new_genesis(timestamp: u64) -> Self {
        Self::new(timestamp, 0, 0, None)
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

    pub fn parent_header(&self) -> Option<&OLBlockHeader> {
        self.parent_header.as_ref()
    }

    /// Computes the blkid of the parent block or returns the null blkid if this
    /// is the genesis block.
    pub fn compute_parent_blkid(&self) -> OLBlockId {
        let Some(_ph) = self.parent_header() else {
            return OLBlockId::null();
        };

        // TODO where did this function go?
        todo!();
    }

    /// Computes the block commitment for the parent block.
    pub fn compute_parent_commitment(&self) -> OLBlockCommitment {
        let Some(ph) = self.parent_header() else {
            return OLBlockCommitment::null();
        };

        // FIXME uhhh this actually does the same destructuring as above but
        // LLVM should be able to figure it out after inlining
        let blkid = self.compute_parent_blkid();
        OLBlockCommitment::new(ph.slot(), blkid)
    }
}

/// Slot execution context.
#[derive(Clone, Debug)]
pub struct SlotExecContext {
    block_context: BlockContext,
    logs: Vec<OLLog>,
}

impl SlotExecContext {
    pub(crate) fn new(block_context: BlockContext) -> Self {
        Self {
            block_context,
            logs: Vec::new(),
        }
    }

    /// Returns a ref to the block context structure.
    pub fn block_context(&self) -> &BlockContext {
        &self.block_context
    }

    pub fn emit_log(&mut self, log: OLLog) {
        self.logs.push(log);
    }

    pub fn emit_logs(&mut self, iter: impl IntoIterator<Item = OLLog>) {
        self.logs.extend(iter);
    }
}
