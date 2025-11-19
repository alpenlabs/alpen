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
    /// Constructs a new instance.
    ///
    /// # Panics
    ///
    /// If there is no parent block but the epoch/slot is nonzero, as that can
    /// only be valid if we're the genesis block.
    pub(crate) fn new(
        timestamp: u64,
        slot: Slot,
        epoch: Epoch,
        parent_header: Option<OLBlockHeader>,
    ) -> Self {
        // Sanity check.
        if parent_header.is_none() && (slot != 0 || epoch != 0) {
            panic!("stf/context: headers are all fucked up");
        }

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

    /// Checks if we're probably at the first block of an epoch.
    ///
    /// This checks if the epoch is greater than the previous header's epoch, or if we have no
    /// parent block (meaning we're probably the genesis block).
    ///
    /// This doesn't behave correctly if the headers are not actually
    /// consistent.
    pub fn is_probably_epoch_initial(&self) -> bool {
        self.parent_header()
            .is_none_or(|ph| self.epoch > ph.epoch())
    }

    /// Constructs an epoch context, for use at an epoch initial.
    ///
    /// # Panics
    ///
    /// If we're "probably not" an epoch initial.
    pub fn to_epoch_initial_context(&self) -> EpochInitialContext {
        assert!(
            self.is_probably_epoch_initial(),
            "stf/context: not epoch initial"
        );
        EpochInitialContext::new(self.epoch(), self.compute_parent_commitment())
    }

    /*
        /// Constructs an epoch terminal context.
        ///
        /// This only makes sense to be called if we're really at an epoch terminal.
        pub fn to_epoch_terminal_context(&self) -> EpochTerminalContext {
            EpochTerminalContext::new(self.epoch(), ExecOutputBuffer::new_empty())
        }
    */
}

/// Epoch-level context for use at the initial.
#[derive(Clone, Debug)]
pub struct EpochInitialContext {
    epoch: Epoch,
    prev_terminal: OLBlockCommitment,
}

impl EpochInitialContext {
    pub(crate) fn new(epoch: Epoch, prev_terminal: OLBlockCommitment) -> Self {
        Self {
            epoch,
            prev_terminal,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn prev_terminal(&self) -> OLBlockCommitment {
        self.prev_terminal
    }
}

/// Epoch-level context for use at the terminal's sealing.
#[derive(Clone, Debug)]
pub struct EpochTerminalContext {
    epoch: Epoch,
    output_buffer: ExecOutputBuffer,
}

impl EpochTerminalContext {
    pub(crate) fn new(epoch: Epoch, output_buffer: ExecOutputBuffer) -> Self {
        Self {
            epoch,
            output_buffer,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn emit_log(&mut self, log: OLLog) {
        self.emit_logs(std::iter::once(log));
    }

    pub fn emit_logs(&mut self, iter: impl IntoIterator<Item = OLLog>) {
        self.output_buffer.emit_logs(iter);
    }

    pub fn into_output(self) -> ExecOutputBuffer {
        self.output_buffer
    }
}

/// Collector for outputs that we can pass around between different contexts.
#[derive(Clone, Debug)]
pub struct ExecOutputBuffer {
    // maybe we'll have stuff other than logs in the future
    logs: Vec<OLLog>,
}

impl ExecOutputBuffer {
    fn new(logs: Vec<OLLog>) -> Self {
        Self { logs }
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    pub fn emit_logs(&mut self, iter: impl IntoIterator<Item = OLLog>) {
        self.logs.extend(iter);
    }

    pub fn into_logs(self) -> Vec<OLLog> {
        self.logs
    }
}

/// Slot execution context.
#[derive(Clone, Debug)]
pub struct SlotExecContext {
    block_context: BlockContext,
    output_buffer: ExecOutputBuffer,
}

impl SlotExecContext {
    pub(crate) fn new(block_context: BlockContext) -> Self {
        Self {
            block_context,
            output_buffer: ExecOutputBuffer::new_empty(),
        }
    }

    /// Returns a ref to the block context structure.
    pub fn block_context(&self) -> &BlockContext {
        &self.block_context
    }

    pub fn emit_log(&mut self, log: OLLog) {
        self.emit_logs(std::iter::once(log));
    }

    pub fn emit_logs(&mut self, iter: impl IntoIterator<Item = OLLog>) {
        self.output_buffer.emit_logs(iter);
    }

    /// Unwraps the context for just the output buffer.
    pub fn into_output(self) -> ExecOutputBuffer {
        self.output_buffer
    }

    /// Converts the slot context into an epoch terminal context, retaining the
    /// output buffer so that we can collect logs and stuff.
    pub fn into_epoch_terminal_context(self) -> EpochTerminalContext {
        let epoch = self.block_context().epoch(); // weird copy
        EpochTerminalContext::new(epoch, self.output_buffer)
    }
}
