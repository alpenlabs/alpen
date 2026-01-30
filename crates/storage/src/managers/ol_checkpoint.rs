//! High-level OL checkpoint interface.

use std::{num::NonZeroUsize, sync::Arc};

use strata_checkpoint_types::EpochSummary;
use strata_db_types::{traits::OLCheckpointDatabase, types::OLCheckpointEntry, DbResult};
use strata_identifiers::Epoch;
use strata_primitives::epoch::EpochCommitment;
use threadpool::ThreadPool;

use crate::{
    cache::CacheTable,
    ops::ol_checkpoint::{Context, OLCheckpointOps},
};

/// Default cache capacity for OL checkpoint entries.
const DEFAULT_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(64).expect("64 is non-zero");

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct OLCheckpointManager {
    ops: OLCheckpointOps,
    checkpoint_cache: CacheTable<Epoch, Option<OLCheckpointEntry>>,
}

impl OLCheckpointManager {
    pub fn new<D: OLCheckpointDatabase + Sync + Send + 'static>(
        pool: ThreadPool,
        db: Arc<D>,
    ) -> Self {
        let ops = Context::new(db.clone()).into_ops(pool);
        let checkpoint_cache = CacheTable::new(DEFAULT_CACHE_CAPACITY);
        Self {
            ops,
            checkpoint_cache,
        }
    }

    /// Stores an OL checkpoint entry for a given epoch.
    pub async fn put_checkpoint_async(
        &self,
        epoch: Epoch,
        entry: OLCheckpointEntry,
    ) -> DbResult<()> {
        self.ops.put_checkpoint_async(epoch, entry.clone()).await?;
        self.checkpoint_cache.insert_async(epoch, Some(entry)).await;
        Ok(())
    }

    /// Inserts an epoch summary retrievable by its epoch commitment.
    pub async fn insert_epoch_summary_async(&self, summary: EpochSummary) -> DbResult<()> {
        self.ops.insert_epoch_summary_async(summary).await
    }

    /// Inserts an epoch summary retrievable by its epoch commitment.
    pub fn insert_epoch_summary_blocking(&self, summary: EpochSummary) -> DbResult<()> {
        self.ops.insert_epoch_summary_blocking(summary)
    }

    /// Gets an epoch summary given an epoch commitment.
    pub async fn get_epoch_summary_async(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<EpochSummary>> {
        self.ops.get_epoch_summary_async(epoch).await
    }

    /// Gets an epoch summary given an epoch commitment.
    pub fn get_epoch_summary_blocking(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<EpochSummary>> {
        self.ops.get_epoch_summary_blocking(epoch)
    }

    /// Gets all commitments for an epoch.
    pub async fn get_epoch_commitments_at_async(
        &self,
        epoch: u64,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops.get_epoch_commitments_at_async(epoch).await
    }

    /// Gets all commitments for an epoch.
    pub fn get_epoch_commitments_at_blocking(&self, epoch: u64) -> DbResult<Vec<EpochCommitment>> {
        self.ops.get_epoch_commitments_at_blocking(epoch)
    }

    /// Gets the index of the last epoch that we have a summary for, if any.
    pub async fn get_last_summarized_epoch_async(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_summarized_epoch_async().await
    }

    /// Gets the index of the last epoch that we have a summary for, if any.
    pub fn get_last_summarized_epoch_blocking(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_summarized_epoch_blocking()
    }

    /// Deletes an epoch summary given an epoch commitment.
    pub async fn del_epoch_summary_async(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_epoch_summary_async(epoch).await
    }

    /// Deletes an epoch summary given an epoch commitment.
    pub fn del_epoch_summary_blocking(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_epoch_summary_blocking(epoch)
    }

    /// Deletes all epoch summaries from the specified epoch onwards (inclusive).
    pub async fn del_epoch_summaries_from_epoch_async(
        &self,
        start_epoch: u64,
    ) -> DbResult<Vec<u64>> {
        self.ops
            .del_epoch_summaries_from_epoch_async(start_epoch)
            .await
    }

    /// Deletes all epoch summaries from the specified epoch onwards (inclusive).
    pub fn del_epoch_summaries_from_epoch_blocking(&self, start_epoch: u64) -> DbResult<Vec<u64>> {
        self.ops
            .del_epoch_summaries_from_epoch_blocking(start_epoch)
    }

    /// Stores an OL checkpoint entry for a given epoch.
    pub fn put_checkpoint_blocking(&self, epoch: Epoch, entry: OLCheckpointEntry) -> DbResult<()> {
        self.ops.put_checkpoint_blocking(epoch, entry.clone())?;
        self.checkpoint_cache.insert_blocking(epoch, Some(entry));
        Ok(())
    }

    /// Retrieves an OL checkpoint entry for a given epoch.
    pub async fn get_checkpoint_async(&self, epoch: Epoch) -> DbResult<Option<OLCheckpointEntry>> {
        self.checkpoint_cache
            .get_or_fetch(&epoch, || self.ops.get_checkpoint_chan(epoch))
            .await
    }

    /// Retrieves an OL checkpoint entry for a given epoch.
    pub fn get_checkpoint_blocking(&self, epoch: Epoch) -> DbResult<Option<OLCheckpointEntry>> {
        self.checkpoint_cache
            .get_or_fetch_blocking(&epoch, || self.ops.get_checkpoint_blocking(epoch))
    }

    /// Gets the last written checkpoint epoch.
    pub async fn get_last_checkpoint_epoch_async(&self) -> DbResult<Option<Epoch>> {
        self.ops.get_last_checkpoint_epoch_async().await
    }

    /// Gets the last written checkpoint epoch.
    pub fn get_last_checkpoint_epoch_blocking(&self) -> DbResult<Option<Epoch>> {
        self.ops.get_last_checkpoint_epoch_blocking()
    }

    /// Gets the next unsigned checkpoint epoch.
    pub async fn get_next_unsigned_checkpoint_epoch_async(&self) -> DbResult<Option<Epoch>> {
        self.ops.get_next_unsigned_checkpoint_epoch_async().await
    }

    /// Gets the next unsigned checkpoint epoch.
    pub fn get_next_unsigned_checkpoint_epoch_blocking(&self) -> DbResult<Option<Epoch>> {
        self.ops.get_next_unsigned_checkpoint_epoch_blocking()
    }

    /// Deletes an OL checkpoint entry for a given epoch.
    pub async fn del_checkpoint_async(&self, epoch: Epoch) -> DbResult<bool> {
        let deleted = self.ops.del_checkpoint_async(epoch).await?;
        self.checkpoint_cache.purge_async(&epoch).await;
        Ok(deleted)
    }

    /// Deletes an OL checkpoint entry for a given epoch.
    pub fn del_checkpoint_blocking(&self, epoch: Epoch) -> DbResult<bool> {
        let deleted = self.ops.del_checkpoint_blocking(epoch)?;
        self.checkpoint_cache.purge_blocking(&epoch);
        Ok(deleted)
    }

    /// Deletes checkpoints from the specified epoch onwards (inclusive).
    pub async fn del_checkpoints_from_epoch_async(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<Epoch>> {
        let deleted = self
            .ops
            .del_checkpoints_from_epoch_async(start_epoch)
            .await?;
        self.checkpoint_cache
            .purge_if_async(|epoch| *epoch >= start_epoch)
            .await;
        Ok(deleted)
    }

    /// Deletes checkpoints from the specified epoch onwards (inclusive).
    pub fn del_checkpoints_from_epoch_blocking(&self, start_epoch: Epoch) -> DbResult<Vec<Epoch>> {
        let deleted = self.ops.del_checkpoints_from_epoch_blocking(start_epoch)?;
        self.checkpoint_cache
            .purge_if_blocking(|epoch| *epoch >= start_epoch);
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use strata_checkpoint_types::EpochSummary;
    use strata_checkpoint_types_ssz::test_utils::checkpoint_payload_strategy;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{
        traits::DatabaseBackend,
        types::{OLCheckpointEntry, OLCheckpointStatus},
    };
    use strata_identifiers::test_utils::{
        buf32_strategy, epoch_strategy, l1_block_commitment_strategy, ol_block_commitment_strategy,
    };
    use threadpool::ThreadPool;
    use tokio::runtime::Runtime;

    use super::*;

    fn setup_manager() -> OLCheckpointManager {
        let pool = ThreadPool::new(1);
        let db = Arc::new(get_test_sled_backend());
        let ol_checkpoint_db = db.ol_checkpoint_db();
        OLCheckpointManager::new(pool, ol_checkpoint_db)
    }

    /// Strategy for generating random [`EpochSummary`] values.
    fn epoch_summary_strategy() -> impl Strategy<Value = EpochSummary> {
        (
            epoch_strategy(),
            ol_block_commitment_strategy(),
            ol_block_commitment_strategy(),
            l1_block_commitment_strategy(),
            buf32_strategy(),
        )
            .prop_map(|(epoch, terminal, prev_terminal, new_l1, final_state)| {
                EpochSummary::new(epoch, terminal, prev_terminal, new_l1, final_state)
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]

        #[test]
        fn proptest_put_and_get_checkpoint_blocking(
            epoch in epoch_strategy(),
            payload in checkpoint_payload_strategy()
        ) {
            let manager = setup_manager();
            let entry = OLCheckpointEntry::new_unsigned(payload.clone());

            manager.put_checkpoint_blocking(epoch, entry).expect("test: put");

            let retrieved = manager
                .get_checkpoint_blocking(epoch)
                .expect("test: get")
                .expect("should exist");

            // Verify retrieved entry matches original
            assert_eq!(
                retrieved.checkpoint.new_tip().l1_height(),
                payload.new_tip().l1_height()
            );
        }

        #[test]
        fn proptest_delete_checkpoint_blocking(
            epoch in epoch_strategy(),
            payload in checkpoint_payload_strategy()
        ) {
            let manager = setup_manager();
            let entry = OLCheckpointEntry::new_unsigned(payload);

            manager.put_checkpoint_blocking(epoch, entry).expect("test: put");

            let deleted = manager.del_checkpoint_blocking(epoch).expect("test: delete");
            prop_assert!(deleted);

            let retrieved = manager.get_checkpoint_blocking(epoch).expect("test: get");
            prop_assert!(retrieved.is_none());
        }

        /// Test: get_last_checkpoint_epoch returns the highest epoch (continuous from 0).
        #[test]
        fn proptest_get_last_checkpoint_epoch_blocking(
            count in 1u32..10u32,
            payload in checkpoint_payload_strategy()
        ) {
            let manager = setup_manager();

            // Initially none
            let last = manager.get_last_checkpoint_epoch_blocking().expect("test: get last");
            prop_assert!(last.is_none());

            // Insert continuous epochs 0..count
            let entry = OLCheckpointEntry::new_unsigned(payload);
            for epoch in 0..count {
                manager.put_checkpoint_blocking(epoch, entry.clone()).expect("put");
            }

            let last = manager
                .get_last_checkpoint_epoch_blocking()
                .expect("test: get last")
                .expect("should exist");

            // Last should be count - 1 (highest inserted epoch)
            prop_assert_eq!(last, count - 1);
        }

        /// Test: del_checkpoints_from_epoch deletes epochs >= cutoff, keeps epochs < cutoff.
        #[test]
        fn proptest_delete_checkpoints_from_epoch_blocking(
            count in 3u32..10u32,
            payload in checkpoint_payload_strategy()
        ) {
            let manager = setup_manager();
            let entry = OLCheckpointEntry::new_unsigned(payload);

            // Insert continuous epochs 0..count
            for epoch in 0..count {
                manager.put_checkpoint_blocking(epoch, entry.clone()).expect("put");
            }

            // Delete from middle epoch onwards
            let cutoff: Epoch = count / 2;
            let deleted = manager
                .del_checkpoints_from_epoch_blocking(cutoff)
                .expect("delete from");

            // Behavior: epochs >= cutoff are deleted
            let expected_deleted_count = count - cutoff;
            prop_assert_eq!(deleted.len() as u32, expected_deleted_count);
            for epoch in cutoff..count {
                prop_assert!(deleted.contains(&epoch));
            }

            // Behavior: epochs < cutoff remain
            for epoch in 0..cutoff {
                prop_assert!(manager.get_checkpoint_blocking(epoch).expect("get").is_some());
            }

            // Behavior: epochs >= cutoff are gone
            for epoch in cutoff..count {
                prop_assert!(manager.get_checkpoint_blocking(epoch).expect("get").is_none());
            }
        }

        #[test]
        fn proptest_status_transition_blocking(
            epoch in epoch_strategy(),
            payload in checkpoint_payload_strategy(),
            intent_index in any::<u64>()
        ) {
            let manager = setup_manager();
            let unsigned = OLCheckpointEntry::new_unsigned(payload.clone());

            manager.put_checkpoint_blocking(epoch, unsigned).expect("test: put unsigned");
            let retrieved = manager
                .get_checkpoint_blocking(epoch)
                .expect("test: get unsigned")
                .expect("should exist");
            prop_assert!(matches!(retrieved.status, OLCheckpointStatus::Unsigned));

            let signed = OLCheckpointEntry::new(payload, OLCheckpointStatus::Signed(intent_index));
            manager.put_checkpoint_blocking(epoch, signed).expect("test: put signed");

            let retrieved = manager
                .get_checkpoint_blocking(epoch)
                .expect("test: get signed")
                .expect("should exist");
            prop_assert!(matches!(
                retrieved.status,
                OLCheckpointStatus::Signed(idx) if idx == intent_index
            ));
        }

    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]

        #[test]
        fn proptest_insert_and_get_epoch_summary_blocking(summary in epoch_summary_strategy()) {
            let manager = setup_manager();
            let commitment = summary.get_epoch_commitment();

            manager
                .insert_epoch_summary_blocking(summary)
                .expect("test: insert");

            let stored = manager
                .get_epoch_summary_blocking(commitment)
                .expect("test: get")
                .expect("test: missing");

            prop_assert_eq!(stored, summary);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]

        #[test]
        fn proptest_put_and_get_checkpoint_async(
            epoch in epoch_strategy(),
            payload in checkpoint_payload_strategy()
        ) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let entry = OLCheckpointEntry::new_unsigned(payload.clone());

                manager.put_checkpoint_async(epoch, entry).await.expect("test: put");

                let retrieved = manager
                    .get_checkpoint_async(epoch)
                    .await
                    .expect("test: get")
                    .expect("should exist");

                assert_eq!(
                    retrieved.checkpoint.new_tip().l1_height(),
                    payload.new_tip().l1_height()
                );
            });
        }

        #[test]
        fn proptest_delete_checkpoint_async(
            epoch in epoch_strategy(),
            payload in checkpoint_payload_strategy()
        ) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let entry = OLCheckpointEntry::new_unsigned(payload);

                manager.put_checkpoint_async(epoch, entry).await.expect("test: put");

                let deleted = manager.del_checkpoint_async(epoch).await.expect("test: delete");
                assert!(deleted);

                let retrieved = manager.get_checkpoint_async(epoch).await.expect("test: get");
                assert!(retrieved.is_none());
            });
        }

        #[test]
        fn proptest_del_checkpoints_from_epoch_async(
            count in 3u32..10u32,
            payload in checkpoint_payload_strategy()
        ) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let entry = OLCheckpointEntry::new_unsigned(payload);

                for epoch in 0..count {
                    manager
                        .put_checkpoint_async(epoch, entry.clone())
                        .await
                        .expect("test: put");
                }

                let cutoff: Epoch = count / 2;
                let deleted = manager
                    .del_checkpoints_from_epoch_async(cutoff)
                    .await
                    .expect("test: delete from");

                let expected_deleted_count = count - cutoff;
                assert_eq!(deleted.len() as u32, expected_deleted_count);

                for epoch in 0..cutoff {
                    assert!(
                        manager
                            .get_checkpoint_async(epoch)
                            .await
                            .expect("test: get")
                            .is_some()
                    );
                }

                for epoch in cutoff..count {
                    assert!(
                        manager
                            .get_checkpoint_async(epoch)
                            .await
                            .expect("test: get")
                            .is_none()
                    );
                }
            });
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]

        #[test]
        fn proptest_insert_and_get_epoch_summary_async(summary in epoch_summary_strategy()) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let commitment = summary.get_epoch_commitment();

                manager
                    .insert_epoch_summary_async(summary)
                    .await
                    .expect("test: insert");

                let stored = manager
                    .get_epoch_summary_async(commitment)
                    .await
                    .expect("test: get")
                    .expect("test: missing");

                assert_eq!(stored, summary);
            });
        }

        #[test]
        fn proptest_del_epoch_summary_async(summary in epoch_summary_strategy()) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let commitment = summary.get_epoch_commitment();

                manager
                    .insert_epoch_summary_async(summary)
                    .await
                    .expect("test: insert");

                let deleted = manager
                    .del_epoch_summary_async(commitment)
                    .await
                    .expect("test: delete");
                assert!(deleted);

                let stored = manager
                    .get_epoch_summary_async(commitment)
                    .await
                    .expect("test: get");
                assert!(stored.is_none());
            });
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]

        #[test]
        fn proptest_get_last_checkpoint_epoch_async(payload in checkpoint_payload_strategy()) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();

                // Initially none
                let last = manager
                    .get_last_checkpoint_epoch_async()
                    .await
                    .expect("get last");
                assert!(last.is_none());

                let entry = OLCheckpointEntry::new_unsigned(payload);

                // Insert epochs 0..5
                for epoch in 0..5u32 {
                    manager
                        .put_checkpoint_async(epoch, entry.clone())
                        .await
                        .expect("put");
                }

                let last = manager
                    .get_last_checkpoint_epoch_async()
                    .await
                    .expect("get last")
                    .expect("should exist");
                assert_eq!(last, 4);
            });
        }

        #[test]
        fn proptest_status_transition_async(
            epoch in epoch_strategy(),
            payload in checkpoint_payload_strategy(),
            intent_index in any::<u64>()
        ) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let unsigned = OLCheckpointEntry::new_unsigned(payload.clone());

                manager
                    .put_checkpoint_async(epoch, unsigned)
                    .await
                    .expect("put unsigned");

                let retrieved = manager
                    .get_checkpoint_async(epoch)
                    .await
                    .expect("get unsigned")
                    .expect("should exist");
                assert!(matches!(retrieved.status, OLCheckpointStatus::Unsigned));

                let signed = OLCheckpointEntry::new(payload, OLCheckpointStatus::Signed(intent_index));
                manager
                    .put_checkpoint_async(epoch, signed)
                    .await
                    .expect("put signed");

                let retrieved = manager
                    .get_checkpoint_async(epoch)
                    .await
                    .expect("get signed")
                    .expect("should exist");
                assert!(matches!(
                    retrieved.status,
                    OLCheckpointStatus::Signed(idx) if idx == intent_index
                ));
            });
        }
    }
}
