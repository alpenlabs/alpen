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
use strata_ol_params::OLParams;
use strata_ol_state_provider::StateProvider;
use strata_predicate::PredicateKey;
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

/// Lifecycle state for a block template.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum BlockTemplateStatus {
    /// The template is awaiting a sequencer signature.
    Pending { template_id: OLBlockId },

    /// The template header was signed, producing the referenced OL block.
    Completed { block: OLBlockCommitment },
}

/// Mutable state for block assembly service (owned by service task).
///
/// Manages pending block templates that have been generated but not yet completed with a
/// signature. Templates are created by `GenerateBlockTemplate` and recorded when
/// `CompleteBlockTemplate` has produced a durably persisted block.
///
/// Templates expire after a configurable TTL. Expired entries are cleaned up during insertion
/// and are treated as absent during lookups.
///
/// # Template Lifecycle
/// 1. Template created via `generate_block_template()` and stored here
/// 2. Template retrieved via `get_pending_block_template()` for signing
/// 3. Template copied for signature validation without cache mutation
/// 4. Template recorded after its block is persisted
/// 5. Template expires and is cleaned up if never completed
#[derive(Debug)]
pub(crate) struct BlockAssemblyState {
    /// Pending templates: template_id -> cached template.
    pub(crate) pending_templates: HashMap<OLBlockId, CachedTemplate>,

    /// Parent block ID -> template status.
    pub(crate) template_status_by_parent: HashMap<OLBlockId, BlockTemplateStatus>,

    /// Time-to-live for cached templates.
    ttl: Duration,
}

impl BlockAssemblyState {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            pending_templates: HashMap::new(),
            template_status_by_parent: HashMap::new(),
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
    ) -> Result<Vec<OLBlockId>, BlockAssemblyError> {
        let mut evicted_template_ids = Vec::new();
        let parent = *template.header().parent_blkid();

        if let Some(BlockTemplateStatus::Completed { block }) =
            self.template_status_by_parent.get(&parent)
        {
            return Err(BlockAssemblyError::TemplateAlreadyCompletedForParent {
                parent,
                block: *block,
            });
        }

        // If we already have a template cached for this parent, evict it to avoid orphans.
        if let Some(BlockTemplateStatus::Pending {
            template_id: old_id,
        }) = self
            .template_status_by_parent
            .insert(parent, BlockTemplateStatus::Pending { template_id })
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
        Ok(evicted_template_ids)
    }

    pub(crate) fn get_template_status_by_parent(
        &self,
        parent_block_id: OLBlockId,
    ) -> Option<BlockTemplateStatus> {
        self.template_status_by_parent
            .get(&parent_block_id)
            .copied()
    }

    /// Keeps completed tombstones above the current parent slot.
    ///
    /// If the current parent is still slot `N - 1`, a completed child at slot `N`
    /// survives and blocks regeneration. Reorgs to lower slots may leave
    /// higher-slot tombstones until the parent slot advances past them.
    pub(crate) fn prune_completed_template_statuses_for_parent(
        &mut self,
        parent_block: OLBlockCommitment,
    ) {
        let parent_slot = parent_block.slot();
        self.template_status_by_parent
            .retain(|_, status| match status {
                BlockTemplateStatus::Pending { .. } => true,
                BlockTemplateStatus::Completed { block } => block.slot() > parent_slot,
            });
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
        let Some(BlockTemplateStatus::Pending { template_id }) =
            self.template_status_by_parent.get(&parent_block_id)
        else {
            return Err(BlockAssemblyError::NoPendingTemplateForParent(
                parent_block_id,
            ));
        };

        self.pending_templates
            .get(template_id)
            .filter(|cached| cached.created_at.elapsed() < self.ttl)
            .map(|cached| cached.template.clone())
            .ok_or(BlockAssemblyError::NoPendingTemplateForParent(
                parent_block_id,
            ))
    }

    /// Records that the block produced from a pending template has been persisted.
    pub(crate) fn record_persisted_block(
        &mut self,
        template_id: OLBlockId,
    ) -> Result<FullBlockTemplate, BlockAssemblyError> {
        let cached = self
            .pending_templates
            .get(&template_id)
            .ok_or(BlockAssemblyError::UnknownTemplateId(template_id))?;

        let template = cached.template.clone();
        let parent = *template.header().parent_blkid();
        let expected_status = BlockTemplateStatus::Pending { template_id };
        if self.template_status_by_parent.get(&parent) != Some(&expected_status) {
            return Err(BlockAssemblyError::UnknownTemplateId(template_id));
        }

        self.pending_templates.remove(&template_id);
        self.template_status_by_parent.insert(
            parent,
            BlockTemplateStatus::Completed {
                block: template.header().compute_block_commitment(),
            },
        );

        Ok(template)
    }

    /// Removes a completed status when it references the provided block.
    pub(crate) fn release_completed_template_status(
        &mut self,
        parent_block_id: OLBlockId,
        block: OLBlockCommitment,
    ) -> bool {
        if self.template_status_by_parent.get(&parent_block_id)
            == Some(&BlockTemplateStatus::Completed { block })
        {
            self.template_status_by_parent.remove(&parent_block_id);
            return true;
        }

        false
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
                if self.template_status_by_parent.get(&parent)
                    == Some(&BlockTemplateStatus::Pending {
                        template_id: *template_id,
                    })
                {
                    self.template_status_by_parent.remove(&parent);
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
    ol_params: Arc<OLParams>,
    blockasm_config: Arc<BlockAssemblyConfig>,
    sequencer_config: SequencerConfig,
    sequencer_predicate: PredicateKey,
    ctx: Arc<BlockAssemblyContext<M, S>>,
    epoch_sealing_policy: E,
    state: BlockAssemblyState,
    epoch_da_tracker: EpochDaTracker,
}

impl<M: MempoolProvider, E: EpochSealingPolicy, S> Debug for BlockasmServiceState<M, E, S> {
    #[expect(clippy::absolute_paths, reason = "qualified Result avoids ambiguity")]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockasmServiceState")
            .field("blockasm_config", &self.blockasm_config)
            .field("sequencer_config", &self.sequencer_config)
            .field("sequencer_predicate", &self.sequencer_predicate)
            .field("ctx", &"<BlockAssemblyContext>")
            .field("state", &self.state)
            .finish()
    }
}

impl<M: MempoolProvider, E: EpochSealingPolicy, S> BlockasmServiceState<M, E, S> {
    /// Create new block assembly service state.
    pub(crate) fn new(
        ol_params: Arc<OLParams>,
        blockasm_config: Arc<BlockAssemblyConfig>,
        sequencer_config: SequencerConfig,
        sequencer_predicate: PredicateKey,
        ctx: Arc<BlockAssemblyContext<M, S>>,
        epoch_sealing_policy: E,
    ) -> Self {
        let ttl = Duration::from_secs(sequencer_config.block_template_ttl_secs);
        Self {
            ol_params,
            blockasm_config,
            sequencer_config,
            sequencer_predicate,
            ctx,
            epoch_sealing_policy,
            state: BlockAssemblyState::new(ttl),
            epoch_da_tracker: EpochDaTracker::new_empty(),
        }
    }

    pub(crate) fn ol_params(&self) -> &OLParams {
        &self.ol_params
    }

    pub(crate) fn sequencer_config(&self) -> &SequencerConfig {
        &self.sequencer_config
    }

    pub(crate) fn sequencer_predicate(&self) -> &PredicateKey {
        &self.sequencer_predicate
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
                None => {
                    rebuild_accumulated_da_upto(
                        parent_blkid,
                        cur_epoch,
                        *self.ol_params().bridge_params(),
                        self.context(),
                    )
                    .await
                }
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
    use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLLog, SignedOLBlockHeader};
    use strata_ol_state_provider::OLStateManagerProviderImpl;
    use strata_ol_state_support_types::EpochDaAccumulator;
    use strata_predicate::PredicateKey;

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

    fn create_test_template_with_parent_and_slot(
        parent: OLBlockId,
        slot: u64,
    ) -> FullBlockTemplate {
        let template = create_test_template_with_parent(parent);
        let header = template.header();
        let header = OLBlockHeader::new(
            header.timestamp(),
            header.flags(),
            slot,
            header.epoch(),
            *header.parent_blkid(),
            *header.body_root(),
            *header.state_root(),
            *header.logs_root(),
        );
        FullBlockTemplate::new(header, template.body().clone())
    }

    async fn build_service_state_with_env(parent_slot: u64) -> (TestServiceState, TestEnv) {
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_parent_slot(parent_slot)
            .with_l1_manifest_height_range(1..=3)
            .build_fixture()
            .await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let state = BlockasmServiceState::new(
            Arc::new(OLParams::default()),
            Arc::new(BlockAssemblyConfig::new(TEST_BLOCK_TEMPLATE_TTL)),
            env.sequencer_config().clone(),
            PredicateKey::always_accept(),
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

        state.insert_template(id, template).unwrap();

        let got = state.get_pending_block_template(id).unwrap();
        assert_eq!(got.get_blockid(), id);
    }

    #[test]
    fn insert_and_get_by_parent() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();

        state.insert_template(id, template).unwrap();

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
    fn record_persisted_block_sets_completed_status() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        let block = template.header().compute_block_commitment();

        state.insert_template(id, template).unwrap();
        let committed = state.record_persisted_block(id).unwrap();
        assert_eq!(committed.get_blockid(), id);

        assert!(state.record_persisted_block(id).is_err());

        // Verify parent lookup fails for signable templates, but the completed tombstone remains.
        assert!(state.get_pending_block_template_by_parent(parent).is_err());
        assert_eq!(
            state.template_status_by_parent.get(&parent),
            Some(&BlockTemplateStatus::Completed { block }),
            "parent index must record the completed block"
        );
    }

    #[test]
    fn insert_template_rejects_completed_parent() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        let block = template.header().compute_block_commitment();

        state.insert_template(id, template).unwrap();
        state.record_persisted_block(id).unwrap();

        let replacement = create_test_template_with_parent(parent);
        let replacement_id = replacement.get_blockid();
        let err = state
            .insert_template(replacement_id, replacement)
            .expect_err("completed parent must reject replacement templates");

        assert!(
            matches!(
                err,
                BlockAssemblyError::TemplateAlreadyCompletedForParent {
                    parent: err_parent,
                    block: err_block,
                } if err_parent == parent && err_block == block
            ),
            "expected TemplateAlreadyCompletedForParent, got: {err:?}"
        );
        assert!(state.get_pending_block_template(replacement_id).is_err());
        assert_eq!(
            state.template_status_by_parent.get(&parent),
            Some(&BlockTemplateStatus::Completed { block }),
            "completed status must not be overwritten"
        );
    }

    #[test]
    fn release_completed_template_status_removes_exact_match() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        let block = template.header().compute_block_commitment();

        state.insert_template(id, template).unwrap();
        state.record_persisted_block(id).unwrap();

        assert!(
            state.release_completed_template_status(parent, block),
            "completed status should be released when parent and block match"
        );
        assert!(
            !state.template_status_by_parent.contains_key(&parent),
            "released completed status should be removed from the parent index"
        );

        let replacement = create_test_template_with_parent(parent);
        let replacement_id = replacement.get_blockid();
        state
            .insert_template(replacement_id, replacement)
            .expect("released parent should accept a replacement template");
        assert_eq!(
            state.template_status_by_parent.get(&parent),
            Some(&BlockTemplateStatus::Pending {
                template_id: replacement_id
            }),
            "released parent should point to the replacement template"
        );
    }

    #[test]
    fn release_completed_template_status_keeps_mismatched_block() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        let block = template.header().compute_block_commitment();
        let other_block =
            OLBlockCommitment::new(block.slot(), OLBlockId::from(Buf32::from([0x55; 32])));

        state.insert_template(id, template).unwrap();
        state.record_persisted_block(id).unwrap();

        assert!(
            !state.release_completed_template_status(parent, other_block),
            "completed status should not be released for a different block"
        );
        assert_eq!(
            state.template_status_by_parent.get(&parent),
            Some(&BlockTemplateStatus::Completed { block }),
            "mismatched release must keep the completed status"
        );
    }

    #[test]
    fn release_completed_template_status_keeps_pending_status() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        let block = template.header().compute_block_commitment();

        state.insert_template(id, template).unwrap();

        assert!(
            !state.release_completed_template_status(parent, block),
            "pending status should not be released by the completed-status path"
        );
        assert_eq!(
            state.template_status_by_parent.get(&parent),
            Some(&BlockTemplateStatus::Pending { template_id: id }),
            "pending status should remain unchanged"
        );
    }

    #[test]
    fn completed_status_releases_when_requested_parent_is_completed_block() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        let block = template.header().compute_block_commitment();

        state.insert_template(id, template).unwrap();
        state.record_persisted_block(id).unwrap();

        state.prune_completed_template_statuses_for_parent(block);

        assert!(
            !state.template_status_by_parent.contains_key(&parent),
            "completed status should be released once generation moves to the completed block"
        );
    }

    #[test]
    fn completed_status_does_not_release_for_same_parent_request() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        let block = template.header().compute_block_commitment();

        state.insert_template(id, template).unwrap();
        state.record_persisted_block(id).unwrap();

        let parent_commitment = OLBlockCommitment::new(block.slot().saturating_sub(1), parent);
        state.prune_completed_template_statuses_for_parent(parent_commitment);

        assert_eq!(
            state.template_status_by_parent.get(&parent),
            Some(&BlockTemplateStatus::Completed { block }),
            "same-parent generation must keep the completed status"
        );
    }

    #[test]
    fn completed_status_cleanup_removes_older_slots() {
        fn parent_id(n: u64) -> OLBlockId {
            let mut bytes = [0u8; 32];
            bytes[..8].copy_from_slice(&n.to_le_bytes());
            OLBlockId::from(Buf32::from(bytes))
        }

        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let parents = [parent_id(1), parent_id(2), parent_id(3)];
        let slots = [9, 10, 11];
        let mut completed = Vec::new();

        for (parent, slot) in parents.into_iter().zip(slots) {
            let template = create_test_template_with_parent_and_slot(parent, slot);
            let id = template.get_blockid();
            let block = template.header().compute_block_commitment();
            state.insert_template(id, template).unwrap();
            state.record_persisted_block(id).unwrap();
            completed.push((parent, block));
        }

        let cleanup_slot = 10;
        let cleanup_parent = OLBlockCommitment::new(cleanup_slot, parent_id(99));

        state.prune_completed_template_statuses_for_parent(cleanup_parent);

        for (parent, block) in completed {
            if block.slot() <= cleanup_slot {
                assert!(
                    !state.template_status_by_parent.contains_key(&parent),
                    "completed status at or before the current parent slot should be removed"
                );
            } else {
                assert!(
                    state.template_status_by_parent.contains_key(&parent),
                    "completed status above the current parent slot should remain"
                );
            }
        }
    }

    #[test]
    fn expired_template_not_returned() {
        let mut state = BlockAssemblyState::new(TEST_BLOCK_TEMPLATE_TTL);
        let template = create_test_template();
        let id = template.get_blockid();
        let parent = *template.header().parent_blkid();

        state.insert_template(id, template).unwrap();

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
        state.insert_template(id1, t1).unwrap();

        let t2 = create_test_template_with_parent(parent);
        let id2 = t2.get_blockid();
        assert_ne!(id1, id2, "templates must have distinct block IDs");
        state.insert_template(id2, t2).unwrap();

        // Old template should be evicted.
        assert!(state.get_pending_block_template(id1).is_err());
        // New template should be present.
        assert!(state.get_pending_block_template(id2).is_ok());
        assert_eq!(
            state.template_status_by_parent.get(&parent),
            Some(&BlockTemplateStatus::Pending { template_id: id2 }),
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
        state.insert_template(id1, t1).unwrap();

        let t2 = create_test_template();
        let id2 = t2.get_blockid();
        let parent2 = *t2.header().parent_blkid();
        assert_ne!(parent1, parent2, "templates must have different parents");
        state.insert_template(id2, t2).unwrap();

        // Backdate the first template to make it expired.
        state
            .set_template_created_at_for_test(id1, Instant::now() - TEST_BLOCK_TEMPLATE_TTL)
            .unwrap();

        // Explicitly call cleanup to remove expired templates.
        state.cleanup_expired_templates();

        // Expired template should be removed from both maps.
        assert!(!state.pending_templates.contains_key(&id1));
        assert!(!state.template_status_by_parent.contains_key(&parent1));

        // Fresh template should still be present.
        assert!(state.pending_templates.contains_key(&id2));
        assert!(state.template_status_by_parent.contains_key(&parent2));
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
            *state.ol_params().bridge_params(),
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
