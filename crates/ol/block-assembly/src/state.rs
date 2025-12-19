//! OL block assembly service state management.

use std::{
    collections::{
        HashMap,
        hash_map::Entry::{Occupied, Vacant},
    },
    sync::Arc,
};

use strata_identifiers::OLBlockId;
use strata_service::ServiceState;

use crate::{
    context::BlockAssemblyContextImpl,
    error::BlockAssemblyError,
    types::{BlockTemplate, FullBlockTemplate},
};

/// Mutable state for block assembly service (owned by service task).
#[derive(Debug)]
pub(crate) struct BlockAssemblyState {
    /// Pending templates: template_id -> full template.
    pending_templates: HashMap<OLBlockId, FullBlockTemplate>,

    /// Parent block ID -> template ID mapping for cache lookups.
    pending_by_parent: HashMap<OLBlockId, OLBlockId>,
}

impl BlockAssemblyState {
    /// Create new block assembly state.
    pub(crate) fn new() -> Self {
        Self {
            pending_templates: HashMap::new(),
            pending_by_parent: HashMap::new(),
        }
    }

    /// Insert a new pending template.
    pub(crate) fn insert_template(&mut self, template_id: OLBlockId, template: FullBlockTemplate) {
        let parent_blockid = *template.header().parent_blkid();
        if let Some(_existing) = self.pending_templates.insert(template_id, template) {
            tracing::warn!("existing pending block template overwritten: {template_id}");
        }
        self.pending_by_parent.insert(parent_blockid, template_id);
    }

    /// Get a pending block template by template ID.
    #[expect(dead_code, reason = "Will be used by handle for template retrieval")]
    pub(crate) fn get_pending_block_template(
        &self,
        template_id: OLBlockId,
    ) -> Result<BlockTemplate, BlockAssemblyError> {
        self.pending_templates
            .get(&template_id)
            .map(BlockTemplate::from_full_ref)
            .ok_or(BlockAssemblyError::UnknownTemplateId(template_id))
    }

    /// Get a pending block template by parent block ID.
    pub(crate) fn get_pending_block_template_by_parent(
        &self,
        parent_block_id: OLBlockId,
    ) -> Result<BlockTemplate, BlockAssemblyError> {
        let template_id = self
            .pending_by_parent
            .get(&parent_block_id)
            .ok_or(BlockAssemblyError::UnknownTemplateId(parent_block_id))?;

        self.pending_templates
            .get(template_id)
            .map(BlockTemplate::from_full_ref)
            .ok_or(BlockAssemblyError::UnknownTemplateId(*template_id))
    }

    /// Remove a template and return it.
    pub(crate) fn remove_template(
        &mut self,
        template_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        match self.pending_templates.entry(template_id) {
            Vacant(_) => Err(BlockAssemblyError::UnknownTemplateId(template_id)),
            Occupied(entry) => {
                let template = entry.remove();
                let parent = *template.header().parent_blkid();
                self.pending_by_parent.remove(&parent);
                Ok(template)
            }
        }
    }
}

/// Combined state for the service (context + mutable state).
pub(crate) struct BlockAssemblyServiceState {
    ctx: Arc<BlockAssemblyContextImpl>,
    state: BlockAssemblyState,
}

impl std::fmt::Debug for BlockAssemblyServiceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockAssemblyServiceState")
            .field("ctx", &"<BlockAssemblyContextImpl>")
            .field("state", &self.state)
            .finish()
    }
}

impl BlockAssemblyServiceState {
    /// Create new block assembly service state.
    pub(crate) fn new(ctx: Arc<BlockAssemblyContextImpl>) -> Self {
        Self {
            ctx,
            state: BlockAssemblyState::new(),
        }
    }

    /// Get a reference to the context.
    pub(crate) fn context(&self) -> &BlockAssemblyContextImpl {
        &self.ctx
    }

    /// Get a mutable reference to the state.
    pub(crate) fn state_mut(&mut self) -> &mut BlockAssemblyState {
        &mut self.state
    }
}

impl ServiceState for BlockAssemblyServiceState {
    fn name(&self) -> &str {
        "ol_block_assembly"
    }
}
