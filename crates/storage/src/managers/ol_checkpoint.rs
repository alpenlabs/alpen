//! High-level OL checkpoint interface.

use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::{traits::OLCheckpointDatabase, types::L1PayloadIntentIndex, DbResult};
use strata_identifiers::{Epoch, EpochCommitment};
use threadpool::ThreadPool;

use crate::ops::ol_checkpoint::{Context, OLCheckpointOps};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct OLCheckpointManager {
    ops: OLCheckpointOps,
}

impl OLCheckpointManager {
    pub fn new<D: OLCheckpointDatabase + Sync + Send + 'static>(
        pool: ThreadPool,
        db: Arc<D>,
    ) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
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
        epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops.get_epoch_commitments_at_async(epoch).await
    }

    /// Gets all commitments for an epoch.
    pub fn get_epoch_commitments_at_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops.get_epoch_commitments_at_blocking(epoch)
    }

    /// Gets the canonical commitment for an epoch index, if any.
    ///
    /// Returns the first commitment for the epoch, which is treated as canonical.
    pub async fn get_canonical_epoch_commitment_at_async(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        let commitments = self.get_epoch_commitments_at_async(epoch).await?;
        Ok(commitments.first().copied())
    }

    /// Gets the canonical commitment for an epoch index, if any.
    ///
    /// Returns the first commitment for the epoch, which is treated as canonical.
    pub fn get_canonical_epoch_commitment_at_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        let commitments = self.get_epoch_commitments_at_blocking(epoch)?;
        Ok(commitments.first().copied())
    }

    /// Gets the index of the last epoch that we have a summary for, if any.
    pub async fn get_last_summarized_epoch_async(&self) -> DbResult<Option<Epoch>> {
        self.ops.get_last_summarized_epoch_async().await
    }

    /// Gets the index of the last epoch that we have a summary for, if any.
    pub fn get_last_summarized_epoch_blocking(&self) -> DbResult<Option<Epoch>> {
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
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_epoch_summaries_from_epoch_async(start_epoch)
            .await
    }

    /// Deletes all epoch summaries from the specified epoch onwards (inclusive).
    pub fn del_epoch_summaries_from_epoch_blocking(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_epoch_summaries_from_epoch_blocking(start_epoch)
    }

    /// Stores an OL checkpoint payload entry by epoch commitment.
    pub async fn put_checkpoint_payload_entry_async(
        &self,
        epoch: EpochCommitment,
        payload: CheckpointPayload,
    ) -> DbResult<()> {
        self.ops
            .put_checkpoint_payload_entry_async(epoch, payload)
            .await
    }

    /// Stores an OL checkpoint payload entry by epoch commitment.
    pub fn put_checkpoint_payload_entry_blocking(
        &self,
        epoch: EpochCommitment,
        payload: CheckpointPayload,
    ) -> DbResult<()> {
        self.ops
            .put_checkpoint_payload_entry_blocking(epoch, payload)
    }

    /// Retrieves an OL checkpoint payload entry by epoch commitment.
    pub async fn get_checkpoint_payload_entry_async(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>> {
        self.ops.get_checkpoint_payload_entry_async(epoch).await
    }

    /// Retrieves an OL checkpoint payload entry by epoch commitment.
    pub fn get_checkpoint_payload_entry_blocking(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>> {
        self.ops.get_checkpoint_payload_entry_blocking(epoch)
    }

    /// Gets the last written checkpoint payload commitment.
    pub async fn get_last_checkpoint_payload_epoch_async(
        &self,
    ) -> DbResult<Option<EpochCommitment>> {
        self.ops.get_last_checkpoint_payload_epoch_async().await
    }

    /// Gets the last written checkpoint payload commitment.
    pub fn get_last_checkpoint_payload_epoch_blocking(&self) -> DbResult<Option<EpochCommitment>> {
        self.ops.get_last_checkpoint_payload_epoch_blocking()
    }

    /// Deletes an OL checkpoint payload entry by epoch commitment.
    pub async fn del_checkpoint_payload_entry_async(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<bool> {
        self.ops.del_checkpoint_payload_entry_async(epoch).await
    }

    /// Deletes an OL checkpoint payload entry by epoch commitment.
    pub fn del_checkpoint_payload_entry_blocking(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_checkpoint_payload_entry_blocking(epoch)
    }

    /// Deletes OL checkpoint payload entries from the specified epoch onwards.
    pub async fn del_checkpoint_payload_entries_from_epoch_async(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_checkpoint_payload_entries_from_epoch_async(start_epoch)
            .await
    }

    /// Deletes OL checkpoint payload entries from the specified epoch onwards.
    pub fn del_checkpoint_payload_entries_from_epoch_blocking(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_checkpoint_payload_entries_from_epoch_blocking(start_epoch)
    }

    /// Stores an OL checkpoint signing entry by epoch commitment.
    pub async fn put_checkpoint_signing_entry_async(
        &self,
        epoch: EpochCommitment,
        payload_intent_idx: L1PayloadIntentIndex,
    ) -> DbResult<()> {
        self.ops
            .put_checkpoint_signing_entry_async(epoch, payload_intent_idx)
            .await
    }

    /// Stores an OL checkpoint signing entry by epoch commitment.
    pub fn put_checkpoint_signing_entry_blocking(
        &self,
        epoch: EpochCommitment,
        payload_intent_idx: L1PayloadIntentIndex,
    ) -> DbResult<()> {
        self.ops
            .put_checkpoint_signing_entry_blocking(epoch, payload_intent_idx)
    }

    /// Retrieves an OL checkpoint signing entry by epoch commitment.
    pub async fn get_checkpoint_signing_entry_async(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<L1PayloadIntentIndex>> {
        self.ops.get_checkpoint_signing_entry_async(epoch).await
    }

    /// Retrieves an OL checkpoint signing entry by epoch commitment.
    pub fn get_checkpoint_signing_entry_blocking(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<L1PayloadIntentIndex>> {
        self.ops.get_checkpoint_signing_entry_blocking(epoch)
    }

    /// Deletes an OL checkpoint signing entry by epoch commitment.
    pub async fn del_checkpoint_signing_entry_async(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<bool> {
        self.ops.del_checkpoint_signing_entry_async(epoch).await
    }

    /// Deletes an OL checkpoint signing entry by epoch commitment.
    pub fn del_checkpoint_signing_entry_blocking(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_checkpoint_signing_entry_blocking(epoch)
    }

    /// Deletes OL checkpoint signing entries from the specified epoch onwards.
    pub async fn del_checkpoint_signing_entries_from_epoch_async(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_checkpoint_signing_entries_from_epoch_async(start_epoch)
            .await
    }

    /// Deletes OL checkpoint signing entries from the specified epoch onwards.
    pub fn del_checkpoint_signing_entries_from_epoch_blocking(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_checkpoint_signing_entries_from_epoch_blocking(start_epoch)
    }

    /// Stores an OL checkpoint L1 ref by epoch commitment.
    pub async fn put_checkpoint_l1_ref_async(
        &self,
        epoch: EpochCommitment,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()> {
        self.ops.put_checkpoint_l1_ref_async(epoch, l1_ref).await
    }

    /// Stores an OL checkpoint L1 ref by epoch commitment.
    pub fn put_checkpoint_l1_ref_blocking(
        &self,
        epoch: EpochCommitment,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()> {
        self.ops.put_checkpoint_l1_ref_blocking(epoch, l1_ref)
    }

    /// Retrieves an OL checkpoint L1 ref by epoch commitment.
    pub async fn get_checkpoint_l1_ref_async(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>> {
        self.ops.get_checkpoint_l1_ref_async(epoch).await
    }

    /// Retrieves an OL checkpoint L1 ref by epoch commitment.
    pub fn get_checkpoint_l1_ref_blocking(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>> {
        self.ops.get_checkpoint_l1_ref_blocking(epoch)
    }

    /// Deletes an OL checkpoint L1 ref by epoch commitment.
    pub async fn del_checkpoint_l1_ref_async(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_checkpoint_l1_ref_async(epoch).await
    }

    /// Deletes an OL checkpoint L1 ref by epoch commitment.
    pub fn del_checkpoint_l1_ref_blocking(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_checkpoint_l1_ref_blocking(epoch)
    }

    /// Deletes OL checkpoint L1 refs from the specified epoch onwards.
    pub async fn del_checkpoint_l1_refs_from_epoch_async(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_checkpoint_l1_refs_from_epoch_async(start_epoch)
            .await
    }

    /// Deletes OL checkpoint L1 refs from the specified epoch onwards.
    pub fn del_checkpoint_l1_refs_from_epoch_blocking(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        self.ops
            .del_checkpoint_l1_refs_from_epoch_blocking(start_epoch)
    }

    /// Gets the next unsigned checkpoint epoch.
    pub async fn get_next_unsigned_checkpoint_epoch_async(&self) -> DbResult<Option<Epoch>> {
        self.ops.get_next_unsigned_checkpoint_epoch_async().await
    }

    /// Gets the next unsigned checkpoint epoch.
    pub fn get_next_unsigned_checkpoint_epoch_blocking(&self) -> DbResult<Option<Epoch>> {
        self.ops.get_next_unsigned_checkpoint_epoch_blocking()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use strata_checkpoint_types::EpochSummary;
    use strata_checkpoint_types_ssz::{
        test_utils::{checkpoint_payload_strategy, create_test_checkpoint_payload},
        CheckpointPayload,
    };
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::DatabaseBackend;
    use strata_identifiers::{
        test_utils::{
            buf32_strategy, epoch_strategy, l1_block_commitment_strategy,
            ol_block_commitment_strategy,
        },
        Epoch, EpochCommitment,
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

    fn checkpoint_epoch_commitment(payload: &CheckpointPayload) -> EpochCommitment {
        EpochCommitment::from_terminal(
            Epoch::from(payload.new_tip().epoch),
            *payload.new_tip().l2_commitment(),
        )
    }
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        #[test]
        fn proptest_payload_roundtrip_blocking(payload in checkpoint_payload_strategy()) {
            let manager = setup_manager();
            let epoch = checkpoint_epoch_commitment(&payload);

            manager
                .put_checkpoint_payload_entry_blocking(epoch, payload.clone())
                .expect("put payload");

            let retrieved = manager
                .get_checkpoint_payload_entry_blocking(epoch)
                .expect("get payload")
                .expect("payload missing");
            prop_assert_eq!(retrieved, payload);
        }

        #[test]
        fn proptest_unsigned_progression_blocking(start in 0u32..1000u32) {
            let manager = setup_manager();

            let payload0 = create_test_checkpoint_payload(start);
            let epoch0 = checkpoint_epoch_commitment(&payload0);
            let payload1 = create_test_checkpoint_payload(start + 1);
            let epoch1 = checkpoint_epoch_commitment(&payload1);

            manager
                .put_checkpoint_payload_entry_blocking(epoch0, payload0)
                .expect("put payload 0");
            manager
                .put_checkpoint_payload_entry_blocking(epoch1, payload1)
                .expect("put payload 1");

            prop_assert_eq!(
                manager
                    .get_next_unsigned_checkpoint_epoch_blocking()
                    .expect("get next unsigned"),
                Some(Epoch::from(start))
            );

            manager
                .put_checkpoint_signing_entry_blocking(epoch0, 42)
                .expect("put signing 0");
            prop_assert_eq!(
                manager
                    .get_next_unsigned_checkpoint_epoch_blocking()
                    .expect("get next unsigned after signing epoch 0"),
                Some(Epoch::from(start + 1))
            );

            manager
                .put_checkpoint_signing_entry_blocking(epoch1, 43)
                .expect("put signing 1");
            prop_assert_eq!(
                manager
                    .get_next_unsigned_checkpoint_epoch_blocking()
                    .expect("get next unsigned after signing all"),
                None
            );

            manager
                .del_checkpoint_signing_entry_blocking(epoch0)
                .expect("delete signing 0");
            prop_assert_eq!(
                manager
                    .get_next_unsigned_checkpoint_epoch_blocking()
                    .expect("get next unsigned after deleting signing"),
                Some(Epoch::from(start))
            );
        }

        #[test]
        fn proptest_payload_delete_removes_signing_entry(
            payload in checkpoint_payload_strategy(),
            intent_idx in any::<u64>(),
        ) {
            let manager = setup_manager();
            let epoch = checkpoint_epoch_commitment(&payload);

            manager
                .put_checkpoint_payload_entry_blocking(epoch, payload)
                .expect("put payload");
            manager
                .put_checkpoint_signing_entry_blocking(epoch, intent_idx)
                .expect("put signing");

            let deleted = manager
                .del_checkpoint_payload_entry_blocking(epoch)
                .expect("delete payload");
            prop_assert!(deleted);
            prop_assert_eq!(
                manager
                    .get_checkpoint_signing_entry_blocking(epoch)
                    .expect("get signing after payload delete"),
                None
            );
        }

        #[test]
        fn proptest_canonical_commitment_is_first_for_epoch(
            epoch in epoch_strategy(),
            terminal_a in ol_block_commitment_strategy(),
            terminal_b in ol_block_commitment_strategy(),
            prev_terminal in ol_block_commitment_strategy(),
            new_l1 in l1_block_commitment_strategy(),
            final_state in buf32_strategy(),
        ) {
            prop_assume!(terminal_a != terminal_b);
            let manager = setup_manager();

            let summary_a = EpochSummary::new(epoch, terminal_a, prev_terminal, new_l1, final_state);
            let summary_b = EpochSummary::new(epoch, terminal_b, prev_terminal, new_l1, final_state);
            let expected = EpochCommitment::from_terminal(epoch, terminal_a.min(terminal_b));

            manager
                .insert_epoch_summary_blocking(summary_a)
                .expect("insert summary a");
            manager
                .insert_epoch_summary_blocking(summary_b)
                .expect("insert summary b");

            let canonical = manager
                .get_canonical_epoch_commitment_at_blocking(epoch)
                .expect("get canonical commitment")
                .expect("canonical commitment missing");
            prop_assert_eq!(canonical, expected);
        }

        #[test]
        fn proptest_payload_roundtrip_async(payload in checkpoint_payload_strategy()) {
            let rt = Runtime::new().expect("create runtime");
            rt.block_on(async {
                let manager = setup_manager();
                let epoch = checkpoint_epoch_commitment(&payload);

                manager
                    .put_checkpoint_payload_entry_async(epoch, payload.clone())
                    .await
                    .expect("put payload async");

                let retrieved = manager
                    .get_checkpoint_payload_entry_async(epoch)
                    .await
                    .expect("get payload async")
                    .expect("payload missing async");
                assert_eq!(retrieved, payload);
            });
        }
    }
}
