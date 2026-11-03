use std::{collections::BTreeSet, future::Future};

use anyhow::bail;
use strata_db_types::{
    l1_writer::{BundledPayloadEntry, IntentEntry, IntentStatus},
    DbResult,
};
use strata_storage::ops::writer::EnvelopeDataOps;
use tracing::*;

pub type PendingIntent = u64;

/// Provides the reads and atomic writes required to bundle intents in order.
pub(crate) trait BundlerStorage: Send + Sync + 'static {
    fn get_intent(&self, idx: u64) -> impl Future<Output = DbResult<Option<IntentEntry>>> + Send;

    fn bundle_intent(
        &self,
        entry: IntentEntry,
        payload: BundledPayloadEntry,
    ) -> impl Future<Output = DbResult<u64>> + Send;
}

impl BundlerStorage for EnvelopeDataOps {
    async fn get_intent(&self, idx: u64) -> DbResult<Option<IntentEntry>> {
        self.get_intent_by_idx_async(idx).await
    }

    async fn bundle_intent(
        &self,
        entry: IntentEntry,
        payload: BundledPayloadEntry,
    ) -> DbResult<u64> {
        self.bundle_intent_payload_async(*entry.intent.commitment(), entry, payload)
            .await
    }
}

/// Bundles pending intents in order, retaining unfinished indexes if an operation fails.
pub(crate) async fn process_unbundled_entries(
    ops: &impl BundlerStorage,
    pending: &mut BTreeSet<PendingIntent>,
) -> anyhow::Result<()> {
    while let Some(&intent_idx) = pending.first() {
        if !is_predecessor_bundled(ops, intent_idx).await? {
            pending.insert(intent_idx - 1); // intent_idx - 1 is safe here as 0 is already checked
            continue;
        }

        bundle_unbundled_intent(ops, intent_idx).await?;
        pending.remove(&intent_idx);
    }
    Ok(())
}

async fn is_predecessor_bundled(ops: &impl BundlerStorage, idx: u64) -> anyhow::Result<bool> {
    if idx == 0 {
        return Ok(true);
    }

    let prev_idx = idx - 1;
    let Some(prev_entry) = ops.get_intent(prev_idx).await? else {
        bail!(
            "inconsistent L1 writer DB: missing predecessor intent idx {prev_idx} before bundling idx {idx}"
        );
    };

    match prev_entry.status {
        IntentStatus::Bundled { .. } => Ok(true),
        IntentStatus::Unbundled => Ok(false),
    }
}

async fn bundle_unbundled_intent(ops: &impl BundlerStorage, intent_idx: u64) -> anyhow::Result<()> {
    let Some(entry) = ops.get_intent(intent_idx).await? else {
        bail!("inconsistent L1 writer DB: pending intent idx {intent_idx} is missing");
    };

    // Check it is actually unbundled, omit if bundled.
    if entry.status != IntentStatus::Unbundled {
        return Ok(());
    }

    // NOTE: In future, the logic to create payload will be different. We need to group
    // intents and create payload entries accordingly
    let payload_entry = BundledPayloadEntry::new_unsigned(entry.payload().clone());

    let intent_commitment = *entry.intent.commitment();
    let payload_idx = ops.bundle_intent(entry, payload_entry).await?;
    info!(
        %intent_commitment,
        intent_idx,
        payload_idx,
        "bundled L1 intent into payload entry"
    );

    Ok(())
}

/// Retrieves unbundled intents since the beginning in ascending order with their intent indexes.
/// This traverses backwards from latest index and breaks once it finds a bundled entry. The
/// processing of unbundled entries [`process_unbundled_entries`] ensures that the entries are
/// bundled *in order*.
pub(crate) fn get_initial_unbundled_entries(
    ops: &EnvelopeDataOps,
) -> anyhow::Result<Vec<PendingIntent>> {
    let mut curr_idx = ops.get_next_intent_idx_blocking()?;
    let mut unbundled = Vec::new();

    while curr_idx > 0 {
        curr_idx -= 1;
        if let Some(intent) = ops.get_intent_by_idx_blocking(curr_idx)? {
            match intent.status {
                IntentStatus::Unbundled => unbundled.push(curr_idx),
                IntentStatus::Bundled { .. } => {
                    // Bundled intent found, no more to scan
                    break;
                }
            }
        } else {
            warn!(%curr_idx, "Could not find expected intent in db");
            break;
        }
    }

    // Reverse the items so that they are in ascending order of index
    unbundled.reverse();

    Ok(unbundled)
}

#[cfg(test)]
mod tests {
    use std::{
        io::ErrorKind,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };

    use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
    use strata_db_types::{
        l1_writer::{BundledPayloadEntry, IntentEntry, IntentStatus},
        DbError,
    };
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::Buf32;
    use strata_service::{AsyncService, Response, TickMsg};
    use tokio::time::advance;

    use super::*;
    use crate::{
        retry::RetryBackoff,
        writer::{
            bundler::service::{BundlerService, BundlerState},
            test_utils::get_envelope_ops,
        },
    };

    fn test_intent(seed: u8) -> PayloadIntent {
        let tag = TagData::new(1, seed, vec![]).expect("test tag is valid");
        let payload = L1Payload::new(vec![vec![seed; 8]], tag).expect("test payload is valid");
        PayloadIntent::new(PayloadDest::L1, Buf32::from([seed; 32]), payload)
    }

    fn put_unbundled_intent(ops: &EnvelopeDataOps, seed: u8) -> (u64, IntentEntry) {
        let intent = test_intent(seed);
        let id = *intent.commitment();
        let entry = IntentEntry::new_unbundled(intent);
        let idx = ops
            .put_intent_entry_blocking(id, entry.clone())
            .expect("test: put intent");
        (idx, entry)
    }

    #[tokio::test]
    async fn processes_missing_unbundled_predecessor_before_later_pending_intent() {
        let ops = get_envelope_ops();
        let (first_idx, first_entry) = put_unbundled_intent(&ops, 1);
        let (second_idx, _) = put_unbundled_intent(&ops, 2);

        process_unbundled_entries(ops.as_ref(), &mut BTreeSet::from([second_idx]))
            .await
            .expect("test: process pending intent");

        let stored_first = ops
            .get_intent_by_idx_blocking(first_idx)
            .expect("test: get first intent")
            .expect("test: first intent exists");
        let stored_second = ops
            .get_intent_by_idx_blocking(second_idx)
            .expect("test: get second intent")
            .expect("test: second intent exists");

        assert_eq!(stored_first.intent, first_entry.intent);
        assert_eq!(stored_first.status, IntentStatus::Bundled { bundle_idx: 0 });
        assert_eq!(
            stored_second.status,
            IntentStatus::Bundled { bundle_idx: 1 }
        );
        assert!(
            get_initial_unbundled_entries(ops.as_ref())
                .expect("test: scan unbundled")
                .is_empty(),
            "restart recovery should not strand an earlier unbundled intent"
        );
    }

    #[tokio::test]
    async fn missing_predecessor_is_fatal_writer_db_inconsistency() {
        let ops = get_envelope_ops();
        let (first_idx, first_entry) = put_unbundled_intent(&ops, 1);
        let (second_idx, _) = put_unbundled_intent(&ops, 2);

        ops.del_intent_entry_blocking(*first_entry.intent.commitment())
            .expect("test: delete predecessor intent");

        let mut state = BundlerState {
            ops,
            unbundled: BTreeSet::from([second_idx]),
            retry: RetryBackoff::new(),
        };
        let err = BundlerService::process_input(&mut state, TickMsg::Tick)
            .await
            .expect_err("missing predecessor should be fatal");

        assert_eq!(first_idx, 0);
        assert!(
            err.to_string()
                .contains("inconsistent L1 writer DB: missing predecessor intent idx 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn startup_scan_returns_indexed_unbundled_tail_in_order() {
        let ops = get_envelope_ops();
        let (first_idx, first_entry) = put_unbundled_intent(&ops, 1);
        let first_payload = BundledPayloadEntry::new_unsigned(first_entry.payload().clone());
        ops.bundle_intent_payload_blocking(
            *first_entry.intent.commitment(),
            first_entry,
            first_payload,
        )
        .expect("test: bundle first intent");
        let (second_idx, _) = put_unbundled_intent(&ops, 2);
        let (third_idx, _) = put_unbundled_intent(&ops, 3);

        let unbundled = get_initial_unbundled_entries(ops.as_ref()).expect("test: scan unbundled");

        assert_eq!(first_idx, 0);
        assert_eq!(unbundled, vec![second_idx, third_idx]);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FailurePoint {
        Read,
        BeforeWrite,
        AfterWrite,
    }

    struct FailingStorage {
        ops: Arc<EnvelopeDataOps>,
        intent: Buf32,
        failure: Mutex<Option<FailurePoint>>,
        reads: AtomicUsize,
    }

    impl FailingStorage {
        fn fail_at(&self, point: FailurePoint) -> DbResult<()> {
            let mut failure = self.failure.lock().unwrap();
            if *failure == Some(point) {
                *failure = None;
                return Err(DbError::Io {
                    kind: ErrorKind::TimedOut,
                    message: "injected storage timeout".into(),
                });
            }

            Ok(())
        }
    }

    impl BundlerStorage for FailingStorage {
        async fn get_intent(&self, idx: u64) -> DbResult<Option<IntentEntry>> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let entry = self.ops.get_intent(idx).await?;
            if entry
                .as_ref()
                .is_some_and(|entry| *entry.intent.commitment() == self.intent)
            {
                self.fail_at(FailurePoint::Read)?;
            }

            Ok(entry)
        }

        async fn bundle_intent(
            &self,
            entry: IntentEntry,
            payload: BundledPayloadEntry,
        ) -> DbResult<u64> {
            let inject_failure = *entry.intent.commitment() == self.intent;
            if inject_failure {
                self.fail_at(FailurePoint::BeforeWrite)?;
            }

            let idx = self.ops.bundle_intent(entry, payload).await?;
            if inject_failure {
                self.fail_at(FailurePoint::AfterWrite)?;
            }

            Ok(idx)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn service_retries_storage_failures_without_losing_or_duplicating_intents() {
        for point in [
            FailurePoint::Read,
            FailurePoint::BeforeWrite,
            FailurePoint::AfterWrite,
        ] {
            let ops = get_envelope_ops();
            let (first_idx, _) = put_unbundled_intent(&ops, 1);
            let (second_idx, second_entry) = put_unbundled_intent(&ops, 2);
            let storage = Arc::new(FailingStorage {
                ops: ops.clone(),
                intent: *second_entry.intent.commitment(),
                failure: Mutex::new(Some(point)),
                reads: AtomicUsize::new(0),
            });
            let mut state = BundlerState {
                ops: storage.clone(),
                unbundled: BTreeSet::from([second_idx]),
                retry: RetryBackoff::new(),
            };

            // The predecessor succeeds; failure must leave the current intent pending.
            let response = BundlerService::process_input(&mut state, TickMsg::Tick)
                .await
                .unwrap();
            assert!(matches!(response, Response::Continue));
            assert_eq!(state.unbundled, BTreeSet::from([second_idx]), "{point:?}");

            // Notifications are retained and deduplicated without bypassing the backoff.
            let (third_idx, _) = put_unbundled_intent(&ops, 3);
            let reads = storage.reads.load(Ordering::Relaxed);
            for idx in [third_idx, second_idx] {
                BundlerService::process_input(&mut state, TickMsg::Msg(idx))
                    .await
                    .unwrap();
            }
            BundlerService::process_input(&mut state, TickMsg::Tick)
                .await
                .unwrap();
            assert_eq!(storage.reads.load(Ordering::Relaxed), reads);
            assert_eq!(state.unbundled, BTreeSet::from([second_idx, third_idx]));

            advance(Duration::from_secs(1)).await;
            BundlerService::process_input(&mut state, TickMsg::Tick)
                .await
                .unwrap();
            assert!(state.unbundled.is_empty());

            // A committed write with a failed response must not allocate another payload.
            assert_eq!(ops.get_next_payload_idx_async().await.unwrap(), 3);
            for (bundle_idx, intent_idx) in
                [first_idx, second_idx, third_idx].into_iter().enumerate()
            {
                let entry = ops.get_intent(intent_idx).await.unwrap().unwrap();
                assert_eq!(
                    entry.status,
                    IntentStatus::Bundled {
                        bundle_idx: bundle_idx as u64
                    }
                );
            }
        }
    }
}
