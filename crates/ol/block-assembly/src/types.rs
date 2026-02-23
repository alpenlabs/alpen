//! Block template types for OL block assembly.

use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf64, OLBlockCommitment, OLBlockId, OLTxId};
use strata_ol_chain_types_new::{OLBlock, OLBlockBody, OLBlockHeader, SignedOLBlockHeader};
use strata_ol_mempool::MempoolTxInvalidReason;

/// Represents a complete block template containing header and body.
///
/// A full block template is an intermediate representation of a block that hasn't been
/// finalized/signed yet. It contains all the components needed to create a complete
/// [`OLBlock`] once signing is complete.
#[derive(Debug, Clone)]
pub struct FullBlockTemplate {
    header: OLBlockHeader,
    body: OLBlockBody,
}

impl FullBlockTemplate {
    /// Creates a new full block template from its components.
    pub fn new(header: OLBlockHeader, body: OLBlockBody) -> Self {
        Self { header, body }
    }

    /// Retrieves the block identifier from the header.
    pub fn get_blockid(&self) -> OLBlockId {
        self.header.compute_blkid()
    }

    /// Returns a reference to the block header.
    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    /// Returns a reference to the block body.
    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }

    /// Accepts signature and finalizes the template into a signed [`OLBlock`].
    pub fn complete_block_template(self, completion: BlockCompletionData) -> OLBlock {
        let FullBlockTemplate { header, body } = self;
        let BlockCompletionData { signature } = completion;
        let signed_header = SignedOLBlockHeader::new(header, signature);

        OLBlock::new(signed_header, body)
    }
}

/// Block template with only sufficient info to be passed for signing.
///
/// Note: `OLBlockHeader` is SSZ-generated and doesn't implement `Serialize`/`Deserialize`.
/// If serialization is needed for RPC, use SSZ encoding instead.
#[derive(Debug, Clone)]
pub struct BlockTemplate {
    header: OLBlockHeader,
}

impl BlockTemplate {
    /// Returns the ID of the template (equivalent to resulting OL block ID).
    pub fn template_id(&self) -> OLBlockId {
        self.header.compute_blkid()
    }

    /// Returns a reference to the OL block header.
    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    /// Create from full block template.
    pub fn from_full_ref(full: &FullBlockTemplate) -> Self {
        Self {
            header: full.header.clone(),
        }
    }
}

/// Sufficient data to complete a [`FullBlockTemplate`] and create a [`OLBlock`].
/// Currently consists of a valid signature for the block from sequencer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct BlockCompletionData {
    signature: Buf64,
}

impl BlockCompletionData {
    /// Create from signature.
    pub fn from_signature(signature: Buf64) -> Self {
        Self { signature }
    }

    /// Returns a reference to signature.
    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }
}

/// Configuration provided by sequencer for the new block to be assembled.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct BlockGenerationConfig {
    parent_block_commitment: OLBlockCommitment,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<u64>,
}

impl BlockGenerationConfig {
    /// Create new instance with provided parent block commitment.
    pub fn new(parent_block_commitment: OLBlockCommitment) -> Self {
        Self {
            parent_block_commitment,
            ts: None,
        }
    }

    /// Update with provided block timestamp.
    pub fn with_ts(mut self, ts: u64) -> Self {
        self.ts = Some(ts);
        self
    }

    /// Return parent block commitment.
    pub fn parent_block_commitment(&self) -> OLBlockCommitment {
        self.parent_block_commitment
    }

    /// Return parent block ID (convenience method).
    pub fn parent_block_id(&self) -> OLBlockId {
        *self.parent_block_commitment.blkid()
    }

    /// Return block timestamp.
    pub fn ts(&self) -> Option<u64> {
        self.ts
    }
}

/// Type alias for a failed mempool transaction with failure reason.
pub(crate) type FailedMempoolTx = (OLTxId, MempoolTxInvalidReason);

/// Result of block template generation including the template and any failed transactions.
#[derive(Debug, Clone)]
pub(crate) struct BlockTemplateResult {
    template: FullBlockTemplate,
    failed_txs: Vec<FailedMempoolTx>,
}

impl BlockTemplateResult {
    /// Create a new block template result.
    pub(crate) fn new(template: FullBlockTemplate, failed_txs: Vec<FailedMempoolTx>) -> Self {
        Self {
            template,
            failed_txs,
        }
    }

    /// Returns the block template.
    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    pub(crate) fn template(&self) -> &FullBlockTemplate {
        &self.template
    }

    /// Consumes self and returns the template.
    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    pub(crate) fn into_template(self) -> FullBlockTemplate {
        self.template
    }

    /// Consumes self and returns both components.
    pub(crate) fn into_parts(self) -> (FullBlockTemplate, Vec<FailedMempoolTx>) {
        (self.template, self.failed_txs)
    }
}
