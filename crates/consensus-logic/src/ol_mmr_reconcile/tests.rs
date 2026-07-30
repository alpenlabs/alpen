use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use strata_acct_types::{append_l1_block_rec_to_mmr, L1BlockRecord, Mmr64};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_types::{DbError, DbResult, MmrId, RawMmrId};
use strata_identifiers::{Epoch, Hash, OLBlockCommitment};
use strata_ol_mmr_index::OLMmrIndexError;
use strata_ol_params::{BridgeParams, OLParams};
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch, MMR_SENTINEL_DUMMY_LEAF_HASH};
use strata_storage::{test_runtime_handle, MmrIndexManager};

use super::{
    reconcile_ol_mmr_index_to_target, MmrIndexReconcileReport, OLMmrReconcileCtx,
    OLMmrReconcileError, OLMmrReconcileTarget,
};
use crate::test_utils::{make_l1_block_commitment, make_ol_block_commitment};

struct MmrReconcileTestCtx {
    mmr_index: MmrIndexManager,
    mmr_truncations: Mutex<Vec<MmrTruncationTarget>>,
    indexing_rollbacks: Mutex<Vec<RecordedIndexingRollback>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MmrTruncationTarget {
    mmr_id: MmrId,
    target_leaf_count: u64,
}

/// An OL-state-indexing rollback the reconciler asked the context to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedIndexingRollback {
    ToEpoch(Epoch),
    ToBlock(Epoch, OLBlockCommitment),
}

#[derive(Clone, Copy, Debug)]
enum TruncateCorruption {
    WrongLeafCount,
    DivergentState,
}

struct FaultyTruncateResultCtx {
    inner: MmrReconcileTestCtx,
    corruption: TruncateCorruption,
}

impl Default for MmrReconcileTestCtx {
    fn default() -> Self {
        let db = get_test_sled_backend();
        Self {
            mmr_index: MmrIndexManager::new(test_runtime_handle(), db.mmr_index_db()),
            mmr_truncations: Mutex::new(Vec::new()),
            indexing_rollbacks: Mutex::new(Vec::new()),
        }
    }
}

impl MmrReconcileTestCtx {
    async fn seed_l1_block_refs_index(&self, records: &[L1BlockRecord]) {
        self.prefill_l1_block_refs_mmr()
            .await
            .expect("prefill L1 block refs MMR");
        for record in records {
            self.append_l1_leaf(Hash::from(record.leaf_hash())).await;
        }
    }

    async fn append_l1_leaf(&self, leaf_hash: Hash) {
        self.mmr_index
            .get_handle(MmrId::L1BlockRefs)
            .append_leaf(leaf_hash)
            .await
            .expect("append L1 block refs leaf");
    }

    async fn l1_leaf_count(&self) -> u64 {
        self.mmr_index
            .get_handle(MmrId::L1BlockRefs)
            .get_leaf_count()
            .await
            .expect("read L1 block refs leaf count")
    }

    fn l1_truncations(&self) -> Vec<u64> {
        self.mmr_truncations
            .lock()
            .expect("test mutex poisoned")
            .iter()
            .filter_map(|truncation| {
                (truncation.mmr_id == MmrId::L1BlockRefs).then_some(truncation.target_leaf_count)
            })
            .collect()
    }

    fn indexing_rollbacks(&self) -> Vec<RecordedIndexingRollback> {
        self.indexing_rollbacks
            .lock()
            .expect("test mutex poisoned")
            .clone()
    }
}

#[async_trait]
impl OLMmrReconcileCtx for MmrReconcileTestCtx {
    async fn prefill_l1_block_refs_mmr(&self) -> DbResult<()> {
        let l1_block_refs = self.mmr_index.get_handle(MmrId::L1BlockRefs);
        if l1_block_refs.get_leaf_count().await? == 0 {
            l1_block_refs
                .append_leaf(MMR_SENTINEL_DUMMY_LEAF_HASH)
                .await?;
        }

        Ok(())
    }

    async fn list_mmr_ids(&self) -> DbResult<Vec<RawMmrId>> {
        let mut ids = self.mmr_index.list_mmr_ids().await?;
        ids.sort();
        Ok(ids)
    }

    async fn get_mmr_leaf_count(&self, mmr_id: &MmrId) -> DbResult<u64> {
        self.mmr_index
            .get_handle(mmr_id.clone())
            .get_leaf_count()
            .await
    }

    async fn get_mmr_state_at(&self, mmr_id: &MmrId, leaf_count: u64) -> DbResult<Mmr64> {
        self.mmr_index
            .get_handle(mmr_id.clone())
            .get_state_at(leaf_count)
            .await
    }

    async fn truncate_mmr_to_leaf_count(
        &self,
        mmr_id: &MmrId,
        target_leaf_count: u64,
    ) -> DbResult<()> {
        self.mmr_truncations
            .lock()
            .expect("test mutex poisoned")
            .push(MmrTruncationTarget {
                mmr_id: mmr_id.clone(),
                target_leaf_count,
            });
        self.mmr_index
            .get_handle(mmr_id.clone())
            .truncate_to_leaf_count(target_leaf_count)
            .await
    }

    async fn reconcile_ol_state_indexing_to_target(
        &self,
        target: &OLMmrReconcileTarget,
    ) -> DbResult<()> {
        let mut rollbacks = self.indexing_rollbacks.lock().expect("test mutex poisoned");
        rollbacks.push(RecordedIndexingRollback::ToEpoch(target.epoch));
        rollbacks.push(RecordedIndexingRollback::ToBlock(
            target.epoch,
            target.block,
        ));
        Ok(())
    }
}

impl FaultyTruncateResultCtx {
    fn new(inner: MmrReconcileTestCtx, corruption: TruncateCorruption) -> Self {
        Self { inner, corruption }
    }

    async fn l1_leaf_count(&self) -> u64 {
        self.inner.l1_leaf_count().await
    }

    fn l1_truncations(&self) -> Vec<u64> {
        self.inner.l1_truncations()
    }

    fn indexing_rollbacks(&self) -> Vec<RecordedIndexingRollback> {
        self.inner.indexing_rollbacks()
    }
}

#[async_trait]
impl OLMmrReconcileCtx for FaultyTruncateResultCtx {
    async fn prefill_l1_block_refs_mmr(&self) -> DbResult<()> {
        self.inner.prefill_l1_block_refs_mmr().await
    }

    async fn list_mmr_ids(&self) -> DbResult<Vec<RawMmrId>> {
        self.inner.list_mmr_ids().await
    }

    async fn get_mmr_leaf_count(&self, mmr_id: &MmrId) -> DbResult<u64> {
        self.inner.get_mmr_leaf_count(mmr_id).await
    }

    async fn get_mmr_state_at(&self, mmr_id: &MmrId, leaf_count: u64) -> DbResult<Mmr64> {
        self.inner.get_mmr_state_at(mmr_id, leaf_count).await
    }

    async fn truncate_mmr_to_leaf_count(
        &self,
        mmr_id: &MmrId,
        target_leaf_count: u64,
    ) -> DbResult<()> {
        match self.corruption {
            TruncateCorruption::WrongLeafCount => {
                let wrong_leaf_count = target_leaf_count
                    .checked_add(1)
                    .ok_or_else(|| DbError::Other("test leaf count overflow".to_string()))?;
                self.inner
                    .truncate_mmr_to_leaf_count(mmr_id, wrong_leaf_count)
                    .await
            }
            TruncateCorruption::DivergentState => {
                let prefix_leaf_count = target_leaf_count.checked_sub(1).ok_or_else(|| {
                    DbError::Other("test target leaf count should be nonzero".to_string())
                })?;
                self.inner
                    .truncate_mmr_to_leaf_count(mmr_id, prefix_leaf_count)
                    .await?;
                self.inner
                    .mmr_index
                    .get_handle(mmr_id.clone())
                    .append_leaf(Hash::from([0xee; 32]))
                    .await?;
                Ok(())
            }
        }
    }

    async fn reconcile_ol_state_indexing_to_target(
        &self,
        target: &OLMmrReconcileTarget,
    ) -> DbResult<()> {
        self.inner
            .reconcile_ol_state_indexing_to_target(target)
            .await
    }
}

fn make_l1_block_record(seed: u8) -> L1BlockRecord {
    L1BlockRecord::new([seed; 32], [seed.wrapping_add(0x80); 32])
}

fn make_target_state_with_l1_records(records: &[L1BlockRecord]) -> OLState {
    let mut state = OLState::from_genesis_params(&OLParams::new_empty(
        make_l1_block_commitment(0, 0),
        BridgeParams::new_with_descriptor_limit(100_000_000, Some(1_000_000_000), 81)
            .expect("valid bridge params"),
    ))
    .expect("valid genesis params");
    let mut l1_block_refs_mmr = state.epoch_state().l1_block_refs_mmr().clone();
    for record in records {
        append_l1_block_rec_to_mmr(&mut l1_block_refs_mmr, record);
    }

    let mut batch = WriteBatch::<OLAccountState>::default();
    batch.epochal_writes_mut().l1_block_refs_mmr = Some(l1_block_refs_mmr);
    state
        .apply_write_batch(batch)
        .expect("apply target L1 refs MMR");
    state
}

fn get_l1_target_mmr(state: &OLState) -> Mmr64 {
    state.epoch_state().l1_block_refs_mmr().clone()
}

fn make_target_commitment(slot: u64) -> OLBlockCommitment {
    make_ol_block_commitment(slot, slot as u8)
}

fn make_reconcile_target(
    block: OLBlockCommitment,
    epoch: Epoch,
    state: Arc<OLState>,
) -> OLMmrReconcileTarget {
    OLMmrReconcileTarget::new(block, epoch, state, BTreeSet::new())
}

#[tokio::test]
async fn test_ahead_index_is_truncated() {
    let ctx = MmrReconcileTestCtx::default();
    let records = [make_l1_block_record(1), make_l1_block_record(2)];
    let target_state = Arc::new(make_target_state_with_l1_records(&records));
    let target_mmr = get_l1_target_mmr(&target_state);
    ctx.seed_l1_block_refs_index(&records).await;
    ctx.append_l1_leaf(Hash::from([0x88; 32])).await;

    let commitment = make_target_commitment(4);
    let report =
        reconcile_ol_mmr_index_to_target(&ctx, make_reconcile_target(commitment, 2, target_state))
            .await
            .expect("reconcile should truncate ahead index");

    assert_eq!(
        report,
        MmrIndexReconcileReport {
            inspected: 1,
            asm_owned_skipped: 0,
            indexes_truncated: 1,
            leaves_removed: 1,
        }
    );
    assert_eq!(ctx.l1_leaf_count().await, target_mmr.num_entries());
    assert_eq!(ctx.l1_truncations(), vec![target_mmr.num_entries()]);
    assert_eq!(
        ctx.indexing_rollbacks(),
        vec![
            RecordedIndexingRollback::ToEpoch(2),
            RecordedIndexingRollback::ToBlock(2, commitment)
        ]
    );
}

#[tokio::test]
async fn test_non_prefix_index_is_rejected_before_truncate() {
    let ctx = MmrReconcileTestCtx::default();
    let target_state = Arc::new(make_target_state_with_l1_records(&[make_l1_block_record(
        1,
    )]));
    let target_mmr = get_l1_target_mmr(&target_state);
    ctx.seed_l1_block_refs_index(&[make_l1_block_record(9)])
        .await;
    ctx.append_l1_leaf(Hash::from([0x88; 32])).await;
    ctx.append_l1_leaf(Hash::from([0x89; 32])).await;

    let err = reconcile_ol_mmr_index_to_target(
        &ctx,
        make_reconcile_target(make_target_commitment(3), 1, target_state),
    )
    .await
    .expect_err("non-prefix index should fail before truncation");

    match err {
        OLMmrReconcileError::TargetPrefixNotInIndex {
            mmr_id,
            target_leaf_count,
            ..
        } => {
            assert_eq!(mmr_id, MmrId::L1BlockRefs);
            assert_eq!(target_leaf_count, target_mmr.num_entries());
        }
        err => panic!("unexpected error: {err:?}"),
    }
    assert_eq!(ctx.l1_leaf_count().await, target_mmr.num_entries() + 2);
    assert!(ctx.l1_truncations().is_empty());
    assert!(ctx.indexing_rollbacks().is_empty());
}

#[tokio::test]
async fn test_bad_final_leaf_count_is_rejected() {
    let inner = MmrReconcileTestCtx::default();
    let target_state = Arc::new(make_target_state_with_l1_records(&[make_l1_block_record(
        1,
    )]));
    let target_mmr = get_l1_target_mmr(&target_state);
    inner
        .seed_l1_block_refs_index(&[make_l1_block_record(1)])
        .await;
    inner.append_l1_leaf(Hash::from([0x88; 32])).await;
    inner.append_l1_leaf(Hash::from([0x89; 32])).await;
    let ctx = FaultyTruncateResultCtx::new(inner, TruncateCorruption::WrongLeafCount);

    let err = reconcile_ol_mmr_index_to_target(
        &ctx,
        make_reconcile_target(make_target_commitment(3), 1, target_state),
    )
    .await
    .expect_err("wrong final leaf count should fail");

    match err {
        OLMmrReconcileError::PostTruncateLeafCountMismatch {
            mmr_id,
            target_leaf_count,
            final_leaf_count,
        } => {
            assert_eq!(mmr_id, MmrId::L1BlockRefs);
            assert_eq!(target_leaf_count, target_mmr.num_entries());
            assert_eq!(final_leaf_count, target_mmr.num_entries() + 1);
        }
        err => panic!("unexpected error: {err:?}"),
    }
    assert_eq!(ctx.l1_leaf_count().await, target_mmr.num_entries() + 1);
    assert_eq!(ctx.l1_truncations(), vec![target_mmr.num_entries() + 1]);
    assert!(ctx.indexing_rollbacks().is_empty());
}

#[tokio::test]
async fn test_bad_final_state_is_rejected() {
    let inner = MmrReconcileTestCtx::default();
    let target_state = Arc::new(make_target_state_with_l1_records(&[make_l1_block_record(
        1,
    )]));
    let target_mmr = get_l1_target_mmr(&target_state);
    inner
        .seed_l1_block_refs_index(&[make_l1_block_record(1)])
        .await;
    inner.append_l1_leaf(Hash::from([0x88; 32])).await;
    inner.append_l1_leaf(Hash::from([0x89; 32])).await;
    let ctx = FaultyTruncateResultCtx::new(inner, TruncateCorruption::DivergentState);

    let err = reconcile_ol_mmr_index_to_target(
        &ctx,
        make_reconcile_target(make_target_commitment(3), 1, target_state),
    )
    .await
    .expect_err("wrong final state should fail");

    match err {
        OLMmrReconcileError::PostTruncateStateMismatch { mmr_id, leaf_count } => {
            assert_eq!(mmr_id, MmrId::L1BlockRefs);
            assert_eq!(leaf_count, target_mmr.num_entries());
        }
        err => panic!("unexpected error: {err:?}"),
    }
    assert_eq!(ctx.l1_leaf_count().await, target_mmr.num_entries());
    assert_eq!(ctx.l1_truncations(), vec![target_mmr.num_entries() - 1]);
    assert!(ctx.indexing_rollbacks().is_empty());
}

#[tokio::test]
async fn test_same_count_state_mismatch_is_rejected() {
    let ctx = MmrReconcileTestCtx::default();
    let target_state = Arc::new(make_target_state_with_l1_records(&[make_l1_block_record(
        1,
    )]));
    let target_mmr = get_l1_target_mmr(&target_state);
    ctx.seed_l1_block_refs_index(&[make_l1_block_record(9)])
        .await;

    let err = reconcile_ol_mmr_index_to_target(
        &ctx,
        make_reconcile_target(make_target_commitment(3), 1, target_state),
    )
    .await
    .expect_err("same-count state mismatch should fail");

    assert!(matches!(
        err,
        OLMmrReconcileError::InvalidIndex(OLMmrIndexError::StateMismatch { .. })
    ));
    assert_eq!(ctx.l1_leaf_count().await, target_mmr.num_entries());
    assert!(ctx.l1_truncations().is_empty());
    assert!(ctx.indexing_rollbacks().is_empty());
}
