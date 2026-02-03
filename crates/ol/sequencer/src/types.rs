//! Types for ol-sequencer, reusing types from block-assembly crate.

// Re-export the block-assembly types
pub use strata_ol_block_assembly::{
    BlockCompletionData, BlockGenerationConfig, BlockTemplate, FullBlockTemplate,
};
use strata_primitives::OLBlockId;

/// Extension trait for BlockTemplate to add convenience methods.
pub trait BlockTemplateExt {
    /// Returns the template ID.
    fn template_id(&self) -> OLBlockId;

    /// Returns the slot number.
    fn slot(&self) -> u64;

    /// Returns the epoch number.
    fn epoch(&self) -> u32;

    /// Returns the timestamp.
    fn timestamp(&self) -> u64;

    /// Returns the parent block ID.
    fn parent(&self) -> OLBlockId;
}

impl BlockTemplateExt for FullBlockTemplate {
    fn template_id(&self) -> OLBlockId {
        self.header().compute_blkid()
    }

    fn slot(&self) -> u64 {
        self.header().slot()
    }

    fn epoch(&self) -> u32 {
        self.header().epoch()
    }

    fn timestamp(&self) -> u64 {
        self.header().timestamp()
    }

    fn parent(&self) -> OLBlockId {
        *self.header().parent_blkid()
    }
}
