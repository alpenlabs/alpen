//! OL block assembly service state management.

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::Arc,
    time::{Duration, Instant},
};

use strata_config::SequencerConfig;
use strata_identifiers::OLBlockId;
use strata_params::{Params, RollupParams};
use strata_service::ServiceState;
use tracing::warn;

use crate::{
    EpochSealingPolicy, MempoolProvider, context::BlockAssemblyContext, error::BlockAssemblyError,
    types::FullBlockTemplate,
};

/// A cached template with its creation time for TTL expiration.
#[derive(Debug, Clone)]
pub(crate) struct CachedTemplate {
    pub(crate) template: FullBlockTemplate,
    pub(crate) created_at: Instant,
}

/// Mutable state for block assembly service (owned by service task).
///
/// Manages pending block templates that have been generated but not yet completed with a
/// signature. Templates are created by `GenerateBlockTemplate` command and removed when
/// `CompleteBlockTemplate` is called with a valid signature.
///
/// Templates expire after a configurable TTL. Expired entries are cleaned up during insertion
/// and are treated as absent during lookups.
///
/// # Template Lifecycle
/// 1. Template created via `generate_block_template()` and stored here
/// 2. Template retrieved via `get_pending_block_template()` for signing
/// 3. Template completed and removed via `remove_template()` after signature validation
/// 4. Template expires and is cleaned up if never completed
#[derive(Debug)]
pub(crate) struct BlockAssemblyState {
    /// Pending templates: template_id -> cached template.
    pub(crate) pending_templates: HashMap<OLBlockId, CachedTemplate>,

    /// Parent block ID -> template ID mapping for cache lookups.
    pub(crate) pending_by_parent: HashMap<OLBlockId, OLBlockId>,

    /// Time-to-live for cached templates.
    ttl: Duration,
}

impl BlockAssemblyState {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            pending_templates: HashMap::new(),
            pending_by_parent: HashMap::new(),
            ttl,
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
        let cached = CachedTemplate {
            template,
            created_at: Instant::now(),
        };
        if self.pending_templates.insert(template_id, cached).is_some() {
            warn!(
                component = "ol_block_assembly",
                %template_id,
                "existing pending block template overwritten"
            );
        }

        self.cleanup_expired_templates();
    }

    /// Gets a pending template by template ID.
    ///
    /// Returns `UnknownTemplateId` if not found or expired.
    pub(crate) fn get_pending_block_template(
        &self,
        template_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        self.pending_templates
            .get(&template_id)
            .filter(|cached| cached.created_at.elapsed() < self.ttl)
            .map(|cached| cached.template.clone())
            .ok_or(BlockAssemblyError::UnknownTemplateId(template_id))
    }

    /// Gets a pending template by parent block ID.
    ///
    /// Returns `NoPendingTemplateForParent` if no mapping exists or the template has expired.
    pub(crate) fn get_pending_block_template_by_parent(
        &self,
        parent_block_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        let template_id = self.pending_by_parent.get(&parent_block_id).ok_or(
            BlockAssemblyError::NoPendingTemplateForParent(parent_block_id),
        )?;

        self.pending_templates
            .get(template_id)
            .filter(|cached| cached.created_at.elapsed() < self.ttl)
            .map(|cached| cached.template.clone())
            .ok_or(BlockAssemblyError::NoPendingTemplateForParent(
                parent_block_id,
            ))
    }

    /// Remove a template and return it.
    pub(crate) fn remove_template(
        &mut self,
        template_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        let cached = self
            .pending_templates
            .remove(&template_id)
            .ok_or(BlockAssemblyError::UnknownTemplateId(template_id))?;

        let parent = *cached.template.header().parent_blkid();
        // Only remove mapping if it still points to this template id.
        if self.pending_by_parent.get(&parent) == Some(&template_id) {
            self.pending_by_parent.remove(&parent);
        }

        Ok(cached.template)
    }

    /// Removes expired entries from both maps.
    pub(crate) fn cleanup_expired_templates(&mut self) {
        let now = Instant::now();
        let ttl = self.ttl;
        let expired_ids: Vec<OLBlockId> = self
            .pending_templates
            .iter()
            .filter(|(_, cached)| now.duration_since(cached.created_at) >= ttl)
            .map(|(id, _)| *id)
            .collect();

        for template_id in &expired_ids {
            if let Some(cached) = self.pending_templates.remove(template_id) {
                let parent = *cached.template.header().parent_blkid();
                if self.pending_by_parent.get(&parent) == Some(template_id) {
                    self.pending_by_parent.remove(&parent);
                }
            }
        }
    }
}

/// Combined state for the service (context + mutable state).
pub(crate) struct BlockasmServiceState<M: MempoolProvider, E: EpochSealingPolicy, S> {
    params: Arc<Params>,
    sequencer_config: SequencerConfig,
    ctx: Arc<BlockAssemblyContext<M, S>>,
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
        ctx: Arc<BlockAssemblyContext<M, S>>,
        epoch_sealing_policy: E,
    ) -> Self {
        let ttl = Duration::from_secs(sequencer_config.block_template_ttl_secs);
        Self {
            params,
            sequencer_config,
            ctx,
            epoch_sealing_policy,
            state: BlockAssemblyState::new(ttl),
        }
    }

    pub(crate) fn rollup_params(&self) -> &RollupParams {
        &self.params.rollup
    }

    pub(crate) fn sequencer_config(&self) -> &SequencerConfig {
        &self.sequencer_config
    }

    pub(crate) fn context(&self) -> &BlockAssemblyContext<M, S> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{
        TEST_BLOCK_TEMPLATE_TTL, create_test_template, create_test_template_with_parent,
    };

    #[test]
    fn insert_and_get_by_id() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();

        state.insert_template(id, template);

        let got = state.get_pending_block_template(id).unwrap();
        assert_eq!(got.get_blockid(), id);
    }

    #[test]
    fn insert_and_get_by_parent() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();

        state.insert_template(id, template);

        let got = state.get_pending_block_template_by_parent(parent).unwrap();
        assert_eq!(got.get_blockid(), id);
    }

    #[test]
    fn get_by_parent_missing_returns_error() {
        let state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let parent = *template.header().parent_blkid();

        assert!(state.get_pending_block_template_by_parent(parent).is_err());
    }

    #[test]
    fn remove_template_succeeds() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();

        state.insert_template(id, template);
        let removed = state.remove_template(id).unwrap();
        assert_eq!(removed.get_blockid(), id);

        // Second removal should fail.
        assert!(state.remove_template(id).is_err());

        // Verify parent lookup also fails (proves both maps cleaned up).
        assert!(state.get_pending_block_template_by_parent(parent).is_err());
    }

    #[test]
    fn expired_template_not_returned() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();

        state.insert_template(id, template);

        // Backdate the entry so it appears expired.
        state.pending_templates.get_mut(&id).unwrap().created_at =
            Instant::now() - TEST_BLOCK_TEMPLATE_TTL;

        assert!(state.get_pending_block_template(id).is_err());
        assert!(state.get_pending_block_template_by_parent(parent).is_err());
    }

    #[test]
    fn overwrite_same_parent_evicts_old() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);

        // Two templates sharing the same parent but with different timestamps → distinct IDs.
        let t1 = create_test_template();
        let parent = *t1.header().parent_blkid();
        let id1 = t1.get_blockid();
        state.insert_template(id1, t1);

        let t2 = create_test_template_with_parent(parent);
        let id2 = t2.get_blockid();
        assert_ne!(id1, id2, "templates must have distinct block IDs");
        state.insert_template(id2, t2);

        // Old template should be evicted.
        assert!(state.get_pending_block_template(id1).is_err());
        // New template should be present.
        assert!(state.get_pending_block_template(id2).is_ok());
    }

    #[test]
    fn cleanup_expired_templates_removes_from_both_maps() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);

        // Insert two templates with different parents.
        let t1 = create_test_template();
        let id1 = t1.get_blockid();
        let parent1 = *t1.header().parent_blkid();
        state.insert_template(id1, t1);

        let t2 = create_test_template();
        let id2 = t2.get_blockid();
        let parent2 = *t2.header().parent_blkid();
        assert_ne!(parent1, parent2, "templates must have different parents");
        state.insert_template(id2, t2);

        // Backdate the first template to make it expired.
        state.pending_templates.get_mut(&id1).unwrap().created_at =
            Instant::now() - TEST_BLOCK_TEMPLATE_TTL;

        // Explicitly call cleanup to remove expired templates.
        state.cleanup_expired_templates();

        // Expired template should be removed from both maps.
        assert!(!state.pending_templates.contains_key(&id1));
        assert!(!state.pending_by_parent.contains_key(&parent1));

        // Fresh template should still be present.
        assert!(state.pending_templates.contains_key(&id2));
        assert!(state.pending_by_parent.contains_key(&parent2));
    }
}
