use serde::{Deserialize, Serialize};
#[cfg(feature = "debug-utils")]
use strata_common::{check_bail_trigger, BAIL_DUTY_SIGN_BLOCK};
use strata_primitives::{buf::Buf64, l2::L2BlockId};
use strata_state::{
    block::{L2Block, L2BlockAccessory, L2BlockBody, L2BlockBundle},
    header::{L2BlockHeader, L2Header, SignedL2BlockHeader},
};

/// Represents a complete block template containing header, body, and accessory data
///
/// A full block template is an intermediate representation of a block that hasn't been
/// finalized/signed yet. It contains all the components needed to create a complete
/// L2BlockBundle once signing is complete.
#[derive(Debug, Clone)]
pub struct FullBlockTemplate {
    header: L2BlockHeader,
    body: L2BlockBody,
    accessory: L2BlockAccessory,
}

impl FullBlockTemplate {
    /// Creates a new full block template from its components.
    pub const fn new(
        header: L2BlockHeader,
        body: L2BlockBody,
        accessory: L2BlockAccessory,
    ) -> Self {
        Self {
            header,
            body,
            accessory,
        }
    }

    /// Retrieves the block identifier from the header.
    pub fn get_blockid(&self) -> L2BlockId {
        self.header.get_blockid()
    }

    /// Returns a reference to the block header.
    pub const fn header(&self) -> &L2BlockHeader {
        &self.header
    }

    /// Accepts signature and finalizes the template into a signed L2BlockBundle.
    pub fn complete_block_template(self, completion: BlockCompletionData) -> L2BlockBundle {
        #[cfg(feature = "debug-utils")]
        check_bail_trigger(BAIL_DUTY_SIGN_BLOCK);
        let FullBlockTemplate {
            header,
            body,
            accessory,
        } = self;
        let BlockCompletionData { signature } = completion;
        let signed_header = SignedL2BlockHeader::new(header, signature);

        let block = L2Block::new(signed_header, body);
        L2BlockBundle::new(block, accessory)
    }
}

/// Block template with only sufficient info to be passed for signing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockTemplate {
    header: L2BlockHeader,
}

impl BlockTemplate {
    /// Returns the ID of the template (equivalent to resulting L2 block ID).
    pub fn template_id(&self) -> L2BlockId {
        self.header.get_blockid()
    }

    /// Returns a reference to the L2 block header.
    pub const fn header(&self) -> &L2BlockHeader {
        &self.header
    }

    /// Create from full block template.
    pub fn from_full_ref(full: &FullBlockTemplate) -> Self {
        Self {
            header: full.header.clone(),
        }
    }
}

/// Sufficient data to complete a [`FullBlockTemplate`] and create a [`L2BlockBundle`].
/// Currently consists of a valid signature for the block from sequencer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockCompletionData {
    signature: Buf64,
}

impl BlockCompletionData {
    /// Create from signature.
    pub const fn from_signature(signature: Buf64) -> Self {
        Self { signature }
    }

    /// Returns a reference to signature.
    pub const fn signature(&self) -> &Buf64 {
        &self.signature
    }
}

/// Configuration provided by sequencer for the new block to be assembled.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct BlockGenerationConfig {
    parent_block_id: L2BlockId,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<u64>,
    // slot: Option<u64>,
    // el payload ?
    epoch_gas_limit: Option<u64>,
}

impl BlockGenerationConfig {
    /// Create new instance with provided parent block id.
    pub fn new(parent_block_id: L2BlockId, epoch_gas_limit: Option<u64>) -> Self {
        Self {
            parent_block_id,
            epoch_gas_limit,
            ..Default::default()
        }
    }

    /// Update with provided block timestamp.
    pub const fn with_ts(mut self, ts: u64) -> Self {
        self.ts = Some(ts);
        self
    }

    /// Return parent block id.
    pub const fn parent_block_id(&self) -> L2BlockId {
        self.parent_block_id
    }

    /// Return block timestamp.
    pub const fn ts(&self) -> Option<u64> {
        self.ts
    }

    /// Return gas limit.
    pub const fn epoch_gas_limit(&self) -> Option<u64> {
        self.epoch_gas_limit
    }
}
