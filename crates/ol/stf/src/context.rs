//! Context types for tracking state across validation.
//!
//! These types are very carefully defined to only be able to expose information
//! that we have available in the different *contexts* that we use them (ie.
//! epoch initialization, transactional processing, epoch sealing, etc.).  If we
//! only cared about regular classical block validation, we could get away with
//! using a single type for everything, but being careful about this is key to
//! ensuring that we don't box ourselves into a design corner where we can't do
//! DA-based state reconstruction.

use std::cell::RefCell;

use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::{Epoch, OLBlockHeader, OLLog, Slot};

use crate::{
    AuxAccumulationCtx,
    output::{ExecOutputBuffer, OutputCtx},
};

/// Simple information about a single block.
///
/// This contains some information that would normally be in the header but that
/// we can know in advance of executing the block.
#[derive(Copy, Clone, Debug)]
pub struct BlockInfo {
    timestamp: u64,
    slot: Slot,
    epoch: Epoch,
}

impl BlockInfo {
    pub(crate) fn new(timestamp: u64, slot: Slot, epoch: Epoch) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
        }
    }

    pub fn new_genesis(timestamp: u64) -> Self {
        Self::new(timestamp, 0, 0)
    }

    pub fn from_header(bh: &OLBlockHeader) -> Self {
        Self::new(bh.timestamp(), bh.slot(), bh.epoch())
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
}

#[derive(Copy, Clone, Debug)]
pub struct EpochInfo {
    terminal_info: BlockInfo,
    prev_terminal: OLBlockCommitment,
}

impl EpochInfo {
    pub fn new(terminal_info: BlockInfo, prev_terminal: OLBlockCommitment) -> Self {
        Self {
            terminal_info,
            prev_terminal,
        }
    }

    pub fn terminal_info(&self) -> BlockInfo {
        self.terminal_info
    }

    pub fn prev_terminal(&self) -> OLBlockCommitment {
        self.prev_terminal
    }

    pub fn epoch(&self) -> Epoch {
        self.terminal_info().epoch()
    }
}

/// Block context relating a block with its header.
#[derive(Copy, Clone, Debug)]
pub struct BlockContext<'b> {
    block_info: &'b BlockInfo,
    parent_header: Option<&'b OLBlockHeader>,
}

impl<'b> BlockContext<'b> {
    /// Constructs a new instance.
    ///
    /// # Panics
    ///
    /// If there is no parent block but the epoch/slot is nonzero, as that can
    /// only be valid if we're the genesis block.
    pub(crate) fn new(block_info: &'b BlockInfo, parent_header: Option<&'b OLBlockHeader>) -> Self {
        // Sanity check genesis context.
        if parent_header.is_none() && (block_info.slot != 0 || block_info.epoch != 0) {
            panic!("stf/context: tried to verify non-genesis with genesis-like context");
        }

        Self {
            block_info,
            parent_header,
        }
    }

    pub fn block_info(&self) -> &BlockInfo {
        self.block_info
    }

    pub fn parent_header(&self) -> Option<&OLBlockHeader> {
        self.parent_header
    }

    pub fn timestamp(&self) -> u64 {
        self.block_info().timestamp()
    }

    pub fn slot(&self) -> u64 {
        self.block_info().slot()
    }

    pub fn epoch(&self) -> u32 {
        self.block_info().epoch()
    }

    /// Computes the blkid of the parent block or returns the null blkid if this
    /// is the genesis block.
    pub fn compute_parent_blkid(&self) -> OLBlockId {
        let Some(ph) = self.parent_header() else {
            return OLBlockId::null();
        };

        // Use the parent header's compute_blkid method
        ph.compute_blkid()
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

    /// Checks if we're the first block of an epoch based on the header flags.
    pub fn is_epoch_initial(&self) -> bool {
        self.parent_header().is_none_or(|ph| ph.is_terminal())
    }

    /// Constructs an epoch context, for use at an epoch initial.
    ///
    /// # Panics
    ///
    /// If we're "probably not" an epoch initial.
    pub fn get_epoch_initial_context(&self) -> EpochInitialContext {
        assert!(self.is_epoch_initial(), "stf/context: not epoch initial");
        EpochInitialContext::new(self.epoch(), self.compute_parent_commitment())
    }
}

/// Limited epoch-level context for use at the initial.
///
/// This can be known without knowing the block.
#[derive(Clone, Debug)]
pub struct EpochInitialContext {
    cur_epoch: Epoch,
    prev_terminal: OLBlockCommitment,
}

impl EpochInitialContext {
    pub(crate) fn new(cur_epoch: Epoch, prev_terminal: OLBlockCommitment) -> Self {
        Self {
            cur_epoch,
            prev_terminal,
        }
    }

    pub fn cur_epoch(&self) -> Epoch {
        self.cur_epoch
    }

    pub fn prev_terminal(&self) -> OLBlockCommitment {
        self.prev_terminal
    }
}

/// Basic execution context which can be used for tracking outputs.
#[derive(Debug)]
pub struct BasicExecContext<'b, A: AuxAccumulationCtx> {
    block_info: BlockInfo,
    output_buffer: &'b ExecOutputBuffer,
    aux_accumulator: &'b A,
}

impl<'b, A: AuxAccumulationCtx> BasicExecContext<'b, A> {
    pub(crate) fn new(
        block_info: BlockInfo,
        output_buffer: &'b ExecOutputBuffer,
        aux_accumulator: &'b A,
    ) -> Self {
        Self {
            block_info,
            output_buffer,
            aux_accumulator,
        }
    }

    fn block_info(&self) -> &BlockInfo {
        &self.block_info
    }

    pub fn output(self) -> &'b ExecOutputBuffer {
        self.output_buffer
    }

    pub fn aux_accumulator(&self) -> &'b A {
        self.aux_accumulator
    }

    pub fn slot(&self) -> Slot {
        self.block_info.slot()
    }

    pub fn epoch(&self) -> Epoch {
        self.block_info.epoch()
    }
}

impl<'b, A: AuxAccumulationCtx> OutputCtx for BasicExecContext<'b, A> {
    fn emit_logs(&self, logs: impl IntoIterator<Item = OLLog>) {
        self.output_buffer.emit_logs(logs);
    }
}

/// Richer execution context which can be used outside of epoch sealing.
#[derive(Clone, Debug)]
pub struct TxExecContext<'b, A: AuxAccumulationCtx> {
    basic_context: &'b BasicExecContext<'b, A>,
    parent_header: Option<&'b OLBlockHeader>,
}

impl<'b, A: AuxAccumulationCtx> TxExecContext<'b, A> {
    pub fn new(
        basic_context: &'b BasicExecContext<'b, A>,
        parent_header: Option<&'b OLBlockHeader>,
    ) -> Self {
        Self {
            basic_context,
            parent_header,
        }
    }

    pub fn basic_context(&self) -> &'b BasicExecContext<'b, A> {
        self.basic_context
    }

    /// Makes a block context from this exec context.
    pub fn to_block_context(&self) -> BlockContext<'b> {
        BlockContext::new(self.basic_context.block_info(), self.parent_header)
    }
}

impl<'b, A: AuxAccumulationCtx> OutputCtx for TxExecContext<'b, A> {
    fn emit_logs(&self, logs: impl IntoIterator<Item = OLLog>) {
        self.basic_context.emit_logs(logs);
    }
}
