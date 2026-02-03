//! Template manager that uses block-assembly service for actual block construction.

use std::{sync::Arc, time::Duration};

use strata_ol_block_assembly::{BlockAssemblyError, BlockasmHandle, FullBlockTemplate};
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::{buf::Buf64, OLBlockCommitment, OLBlockId};
use strata_storage::NodeStorage;
use tokio::sync::Mutex;
use tracing::*;

use crate::{
    cache::TemplateCache,
    error::Error,
    types::{BlockCompletionData, BlockGenerationConfig},
    BlockTemplateExt,
};

/// Manages block template generation and completion using the block-assembly service.
///
/// This is a thin wrapper around BlockasmHandle that adds:
/// - TTL-based template caching
/// - Cleanup on block creation
/// - Simplified interface for sequencer duties
pub struct TemplateManager {
    /// Template cache with TTL-based expiration.
    cache: Arc<Mutex<TemplateCache>>,

    /// Block assembly service handle.
    block_assembly: Arc<BlockasmHandle>,

    /// Storage for fetching parent block slots.
    storage: Arc<NodeStorage>,
}

impl TemplateManager {
    /// Creates a new template manager.
    pub fn new(
        block_assembly: Arc<BlockasmHandle>,
        storage: Arc<NodeStorage>,
        ttl: Duration,
    ) -> Self {
        Self {
            cache: Arc::new(Mutex::new(TemplateCache::new(ttl))),
            block_assembly,
            storage,
        }
    }

    /// Generates a block template for the given parent.
    ///
    /// Returns a cached template if available and not expired,
    /// otherwise delegates to the block-assembly service.
    pub async fn generate_template(
        &self,
        parent_block_id: OLBlockId,
    ) -> Result<FullBlockTemplate, Error> {
        // Try get from cache
        {
            let mut cache = self.cache.lock().await;
            if let Some(template) = cache.get_by_parent(&parent_block_id) {
                return Ok(template);
            }
        }
        // Get parent block to fetch its slot for BlockCommitment
        let parent_block = self
            .storage
            .l2()
            .get_block_data_async(&parent_block_id)
            .await
            .map_err(Error::Database)?
            .ok_or(Error::UnknownBlock(parent_block_id))?;

        let parent_slot = parent_block.header().header().slot();

        // Create BlockGenerationConfig with proper commitment
        let config =
            BlockGenerationConfig::new(OLBlockCommitment::new(parent_slot, parent_block_id));

        // Generate template using block-assembly service
        let template = self
            .block_assembly
            .generate_block_template(config)
            .await
            .map_err(to_template_error)?;

        // Cache it
        {
            let mut cache = self.cache.lock().await;
            cache.insert(template.clone());
            debug!(
                "Cached template {} for parent {}",
                template.template_id(),
                parent_block_id
            );
        }

        Ok(template)
    }

    /// Completes a template with a signature and stores the resulting block.
    pub async fn complete_template(
        &self,
        template_id: OLBlockId,
        signature: Buf64,
    ) -> Result<OLBlock, Error> {
        let completion = BlockCompletionData::from_signature(signature);

        // Complete using block-assembly service
        let block = self
            .block_assembly
            .complete_block_template(template_id, completion)
            .await
            .map_err(to_template_error)?;

        info!(
            "Completed block {} at slot {}",
            block.header().compute_blkid(),
            block.header().slot()
        );

        Ok(block)
    }

    /// Returns the current size of the cache (for monitoring).
    pub async fn cache_size(&self) -> usize {
        self.cache.lock().await.len()
    }
}

/// Maps BlockAssemblyError to our Error type.
fn to_template_error(err: BlockAssemblyError) -> Error {
    match err {
        BlockAssemblyError::UnknownTemplateId(id) => Error::UnknownTemplate(id),
        BlockAssemblyError::InvalidSignature(id) => Error::InvalidSignature(id),
        BlockAssemblyError::Database(db_err) => Error::Database(db_err),
        BlockAssemblyError::RequestChannelClosed | BlockAssemblyError::ResponseChannelClosed => {
            Error::ConsensusChannelClosed
        }
        BlockAssemblyError::TimestampTooEarly(ts) => {
            Error::InvalidConfig(format!("Timestamp too early: {}", ts))
        }
        other => Error::InvalidConfig(format!("Block assembly error: {:?}", other)),
    }
}
