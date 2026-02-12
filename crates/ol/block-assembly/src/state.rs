//! OL block assembly service state management.

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::Arc,
};

use strata_config::SequencerConfig;
use strata_identifiers::OLBlockId;
use strata_params::{Params, RollupParams};
use strata_service::ServiceState;
use tracing::warn;

use crate::{
    EpochSealingPolicy, MempoolProvider, context::BlockAssemblyContextImpl,
    error::BlockAssemblyError, types::FullBlockTemplate,
};

/// Mutable state for block assembly service (owned by service task).
///
/// Manages pending block templates that have been generated but not yet completed with a
/// signature. Templates are created by `GenerateBlockTemplate` command and removed when
/// `CompleteBlockTemplate` is called with a valid signature.
///
/// # Template Lifecycle
/// 1. Template created via `generate_block_template()` and stored here
/// 2. Template retrieved via `get_pending_block_template()` for signing
/// 3. Template completed and removed via `remove_template()` after signature validation
///
/// TODO(STR-2073): Add TTL/cleanup mechanism for templates that are never completed.
#[derive(Debug)]
pub(crate) struct BlockAssemblyState {
    /// Pending templates: template_id -> full template.
    pending_templates: HashMap<OLBlockId, FullBlockTemplate>,

    /// Parent block ID -> template ID mapping for cache lookups.
    pending_by_parent: HashMap<OLBlockId, OLBlockId>,
}

impl BlockAssemblyState {
    pub(crate) fn new() -> Self {
        Self {
            pending_templates: HashMap::new(),
            pending_by_parent: HashMap::new(),
        }
    }

    /// Insert a new pending template.
    ///
    /// Invariant: at most one pending template per parent.
    pub(crate) fn insert_template(&mut self, template_id: OLBlockId, template: FullBlockTemplate) {
        let parent = *template.header().parent_blkid();

        // If we already have a template cached for this parent, evict it to avoid orphans.
        if let Some(old_id) = self.pending_by_parent.insert(parent, template_id)
            && old_id != template_id
        {
            self.pending_templates.remove(&old_id);
        }

        // Insert/overwrite the template itself.
        if self
            .pending_templates
            .insert(template_id, template)
            .is_some()
        {
            warn!(
                component = "ol_block_assembly",
                %template_id,
                "existing pending block template overwritten"
            );
        }
    }

    pub(crate) fn get_pending_block_template(
        &self,
        template_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        self.pending_templates
            .get(&template_id)
            .cloned()
            .ok_or(BlockAssemblyError::UnknownTemplateId(template_id))
    }

    pub(crate) fn get_pending_block_template_by_parent(
        &self,
        parent_block_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        let template_id = self.pending_by_parent.get(&parent_block_id).ok_or(
            BlockAssemblyError::NoPendingTemplateForParent(parent_block_id),
        )?;

        self.pending_templates
            .get(template_id)
            .cloned()
            .ok_or(BlockAssemblyError::UnknownTemplateId(*template_id))
    }

    /// Remove a template and return it.
    pub(crate) fn remove_template(
        &mut self,
        template_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        let template = self
            .pending_templates
            .remove(&template_id)
            .ok_or(BlockAssemblyError::UnknownTemplateId(template_id))?;

        let parent = *template.header().parent_blkid();
        // Only remove mapping if it still points to this template id.
        if self.pending_by_parent.get(&parent) == Some(&template_id) {
            self.pending_by_parent.remove(&parent);
        }

        Ok(template)
    }
}

/// Combined state for the service (context + mutable state).
pub(crate) struct BlockasmServiceState<M: MempoolProvider, E: EpochSealingPolicy, S> {
    params: Arc<Params>,
    sequencer_config: SequencerConfig,
    ctx: Arc<BlockAssemblyContextImpl<M, S>>,
    epoch_sealing_policy: E,
    state: BlockAssemblyState,
}

impl<M: MempoolProvider, E: EpochSealingPolicy, S> Debug for BlockasmServiceState<M, E, S> {
    #[expect(clippy::absolute_paths, reason = "qualified Result avoids ambiguity")]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockasmServiceState")
            .field("params", &"<Params>")
            .field("sequencer_config", &self.sequencer_config)
            .field("ctx", &"<BlockAssemblyContext>")
            .field("state", &self.state)
            .finish()
    }
}

impl<M: MempoolProvider, E: EpochSealingPolicy, S> BlockasmServiceState<M, E, S> {
    /// Create new block assembly service state.
    pub(crate) fn new(
        params: Arc<Params>,
        sequencer_config: SequencerConfig,
        ctx: Arc<BlockAssemblyContextImpl<M, S>>,
        epoch_sealing_policy: E,
    ) -> Self {
        Self {
            params,
            sequencer_config,
            ctx,
            epoch_sealing_policy,
            state: BlockAssemblyState::new(),
        }
    }

    pub(crate) fn rollup_params(&self) -> &RollupParams {
        &self.params.rollup
    }

    pub(crate) fn sequencer_config(&self) -> &SequencerConfig {
        &self.sequencer_config
    }

    pub(crate) fn context(&self) -> &BlockAssemblyContextImpl<M, S> {
        self.ctx.as_ref()
    }

    pub(crate) fn epoch_sealing_policy(&self) -> &E {
        &self.epoch_sealing_policy
    }

    pub(crate) fn state_mut(&mut self) -> &mut BlockAssemblyState {
        &mut self.state
    }
}

impl<M: MempoolProvider, E: EpochSealingPolicy, S: Send + Sync + 'static> ServiceState
    for BlockasmServiceState<M, E, S>
{
    fn name(&self) -> &str {
        "ol_block_assembly"
    }
}
