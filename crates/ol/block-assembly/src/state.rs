//! OL block assembly service state management.

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::Arc,
    time::{Duration, Instant},
};

use strata_config::{BlockAssemblyConfig, SequencerConfig};
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ledger_types::{IAccountStateMut, IStateAccessorMut};
use strata_ol_state_provider::StateProvider;
use strata_params::{Params, RollupParams};
use strata_service::ServiceState;
use tracing::warn;

use crate::{
    BlockAssemblyAnchorContext, BlockAssemblyStateAccess, EpochSealingPolicy, MempoolProvider,
    context::BlockAssemblyContext,
    da_tracker::{AccumulatedDaData, EpochDaTracker, rebuild_accumulated_da_upto},
    error::BlockAssemblyError,
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
    ///
    /// Returns template IDs evicted while inserting (same-parent replacement and TTL cleanup).
    pub(crate) fn insert_template(
        &mut self,
        template_id: OLBlockId,
        template: FullBlockTemplate,
    ) -> Vec<OLBlockId> {
        let mut evicted_template_ids = Vec::new();
        let parent = *template.header().parent_blkid();

        // If we already have a template cached for this parent, evict it to avoid orphans.
        if let Some(old_id) = self.pending_by_parent.insert(parent, template_id)
            && old_id != template_id
        {
            self.pending_templates.remove(&old_id);
            evicted_template_ids.push(old_id);
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

        evicted_template_ids.extend(self.cleanup_expired_templates());
        evicted_template_ids
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

    /// Removes expired entries from both maps and returns removed template IDs.
    pub(crate) fn cleanup_expired_templates(&mut self) -> Vec<OLBlockId> {
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
        expired_ids
    }

    /// Sets a cached template creation time for expiry tests.
    #[cfg(test)]
    pub(crate) fn set_template_created_at_for_test(
        &mut self,
        template_id: OLBlockId,
        created_at: Instant,
    ) -> Result<(), BlockAssemblyError> {
        let cached = self
            .pending_templates
            .get_mut(&template_id)
            .ok_or(BlockAssemblyError::UnknownTemplateId(template_id))?;
        cached.created_at = created_at;
        Ok(())
    }
}

/// Combined state for the service (context + mutable state).
pub(crate) struct BlockasmServiceState<M: MempoolProvider, E: EpochSealingPolicy, S> {
    params: Arc<Params>,
    blockasm_config: Arc<BlockAssemblyConfig>,
    sequencer_config: SequencerConfig,
    ctx: Arc<BlockAssemblyContext<M, S>>,
    epoch_sealing_policy: E,
    state: BlockAssemblyState,
    epoch_da_tracker: EpochDaTracker,
}

impl<M: MempoolProvider, E: EpochSealingPolicy, S> Debug for BlockasmServiceState<M, E, S> {
    #[expect(clippy::absolute_paths, reason = "qualified Result avoids ambiguity")]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockasmServiceState")
            .field("params", &"<Params>")
            .field("blockasm_config", &self.blockasm_config)
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
        blockasm_config: Arc<BlockAssemblyConfig>,
        sequencer_config: SequencerConfig,
        ctx: Arc<BlockAssemblyContext<M, S>>,
        epoch_sealing_policy: E,
    ) -> Self {
        let ttl = Duration::from_secs(sequencer_config.block_template_ttl_secs);
        Self {
            params,
            blockasm_config,
            sequencer_config,
            ctx,
            epoch_sealing_policy,
            state: BlockAssemblyState::new(ttl),
            epoch_da_tracker: EpochDaTracker::new_empty(),
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

    pub(crate) fn epoch_da_tracker(&self) -> &EpochDaTracker {
        &self.epoch_da_tracker
    }

    pub(crate) fn epoch_da_tracker_mut(&mut self) -> &mut EpochDaTracker {
        &mut self.epoch_da_tracker
    }
}

impl<M, E, S> BlockasmServiceState<M, E, S>
where
    M: MempoolProvider + Send + Sync + 'static,
    E: EpochSealingPolicy,
    S: StateProvider + Send + Sync + 'static,
    S::State: BlockAssemblyStateAccess,
    <<S::State as IStateAccessorMut>::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut:
        Clone,
{
    /// Resolves accumulated DA upto a block: returns cached data, rebuilds by re-executing epoch
    /// blocks, or creates fresh data if parent is terminal.
    pub(crate) async fn fetch_epoch_da_until_parent(
        &self,
        parent_blkid: OLBlockCommitment,
    ) -> Result<AccumulatedDaData, BlockAssemblyError> {
        let parent_blk = self
            .context()
            .fetch_ol_block(parent_blkid.blkid)
            .await?
            .ok_or(BlockAssemblyError::BlockNotFound(parent_blkid.blkid))?;

        let parent_header = parent_blk.header();

        // If parent block is terminal then we are in the new epoch and thus start afresh.
        if parent_header.is_terminal() {
            Ok(AccumulatedDaData::new_empty())
        } else {
            // Parent is not terminal, so we try to fetch accumulated da for the epoch.
            let cur_epoch = parent_header.epoch();
            match self
                .epoch_da_tracker()
                .get_accumulated_da(parent_blkid.blkid)
            {
                Some(da) => Ok(da.clone()),
                None => rebuild_accumulated_da_upto(parent_blkid, cur_epoch, self.context()).await,
            }
        }
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
    use std::sync::Arc;

    use strata_config::BlockAssemblyConfig;
    use strata_identifiers::{AccountSerial, Buf32, Buf64};
    use strata_ol_chain_types_new::{OLBlock, OLLog, SignedOLBlockHeader};
    use strata_ol_state_provider::OLStateManagerProviderImpl;
    use strata_ol_state_support_types::EpochDaAccumulator;
    use strata_test_utils_l2::gen_params;

    use super::*;
    use crate::{
        FixedSlotSealing,
        block_assembly::generate_block_template_inner,
        da_tracker::AccumulatedDaData,
        test_utils::{
            MockMempoolProvider, TEST_BLOCK_TEMPLATE_TTL, TestEnv, TestStorageFixtureBuilder,
            create_test_template, create_test_template_with_parent,
        },
        types::BlockGenerationConfig,
    };

    type TestServiceState = BlockasmServiceState<
        Arc<MockMempoolProvider>,
        FixedSlotSealing,
        OLStateManagerProviderImpl,
    >;

    fn sample_accumulated_da() -> AccumulatedDaData {
        AccumulatedDaData::new(
            EpochDaAccumulator::default(),
            vec![OLLog::new(AccountSerial::from(4242_u32), vec![1, 2, 3])],
        )
    }

    async fn build_service_state_with_env(parent_slot: u64) -> (TestServiceState, TestEnv) {
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_parent_slot(parent_slot)
            .with_l1_manifest_height_range(1..=3)
            .build_fixture()
            .await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let state = BlockasmServiceState::new(
            Arc::new(gen_params()),
            Arc::new(BlockAssemblyConfig::new(TEST_BLOCK_TEMPLATE_TTL)),
            env.sequencer_config().clone(),
            env.ctx_arc(),
            env.epoch_sealing_policy().clone(),
        );

        (state, env)
    }

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
        assert!(
            !state.pending_by_parent.contains_key(&parent),
            "parent index must be cleared when template is removed"
        );
    }

    #[test]
    fn expired_template_not_returned() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();

        state.insert_template(id, template);

        // Backdate the entry so it appears expired.
        state
            .set_template_created_at_for_test(id, Instant::now() - TEST_BLOCK_TEMPLATE_TTL)
            .unwrap();

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
        assert_eq!(
            state.pending_by_parent.get(&parent),
            Some(&id2),
            "parent index must point to the newest template id"
        );
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
        state
            .set_template_created_at_for_test(id1, Instant::now() - TEST_BLOCK_TEMPLATE_TTL)
            .unwrap();

        // Explicitly call cleanup to remove expired templates.
        state.cleanup_expired_templates();

        // Expired template should be removed from both maps.
        assert!(!state.pending_templates.contains_key(&id1));
        assert!(!state.pending_by_parent.contains_key(&parent1));

        // Fresh template should still be present.
        assert!(state.pending_templates.contains_key(&id2));
        assert!(state.pending_by_parent.contains_key(&parent2));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fetch_da_terminal_parent_empty() {
        let (mut state, env) = build_service_state_with_env(0).await;

        // Even if tracker has an entry, terminal-parent path must return fresh empty DA.
        state
            .epoch_da_tracker_mut()
            .set_accumulated_da(*env.parent_commitment().blkid(), sample_accumulated_da());

        let parent_block = state
            .context()
            .fetch_ol_block(*env.parent_commitment().blkid())
            .await
            .expect("fetch should succeed")
            .expect("parent block should exist");
        assert!(
            parent_block.header().is_terminal(),
            "test setup requires terminal parent"
        );

        let da = state
            .fetch_epoch_da_until_parent(env.parent_commitment())
            .await
            .expect("terminal parent should return empty DA");
        assert!(
            da.logs().is_empty(),
            "terminal-parent path must reset accumulated DA"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fetch_da_cache_hit() {
        let (mut state, env) = build_service_state_with_env(1).await;

        let parent_block = state
            .context()
            .fetch_ol_block(*env.parent_commitment().blkid())
            .await
            .expect("fetch should succeed")
            .expect("parent block should exist");
        assert!(
            !parent_block.header().is_terminal(),
            "test setup requires non-terminal parent"
        );

        let cached_da = sample_accumulated_da();
        let expected_logs = cached_da.logs().to_vec();
        state
            .epoch_da_tracker_mut()
            .set_accumulated_da(*env.parent_commitment().blkid(), cached_da);

        let da = state
            .fetch_epoch_da_until_parent(env.parent_commitment())
            .await
            .expect("cache-hit path should succeed");
        assert_eq!(
            da.logs(),
            expected_logs.as_slice(),
            "cache-hit path should return tracker data without rebuild"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fetch_da_cache_miss_rebuild() {
        let (state, env) = build_service_state_with_env(0).await;

        let config = BlockGenerationConfig::new(env.parent_commitment());
        let result = generate_block_template_inner(
            state.context(),
            env.epoch_sealing_policy(),
            env.sequencer_config(),
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("child block generation should succeed");
        let child_template = result.into_template();
        assert!(
            !child_template.header().is_terminal(),
            "test setup requires non-terminal child for cache-miss rebuild"
        );

        let child_commitment = OLBlockCommitment::new(
            child_template.header().slot(),
            child_template.header().compute_blkid(),
        );
        let signed_header =
            SignedOLBlockHeader::new(child_template.header().clone(), Buf64::zero());
        let child_block = OLBlock::new(signed_header, child_template.body().clone());
        env.put_block(child_block).await;

        let da = state
            .fetch_epoch_da_until_parent(child_commitment)
            .await
            .expect("cache-miss rebuild should succeed");
        assert!(
            da.logs().is_empty(),
            "empty-child rebuild should produce empty logs"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fetch_da_missing_parent_not_found() {
        let (state, env) = build_service_state_with_env(0).await;
        let missing = OLBlockCommitment::new(
            env.parent_commitment().slot() + 100,
            OLBlockId::from(Buf32::from([0xff_u8; 32])),
        );

        let err = state
            .fetch_epoch_da_until_parent(missing)
            .await
            .expect_err("missing parent should fail");
        assert!(
            matches!(err, BlockAssemblyError::BlockNotFound(_)),
            "expected BlockNotFound(_), got: {err:?}"
        );
    }
}
