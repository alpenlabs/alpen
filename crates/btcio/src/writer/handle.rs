use std::sync::Arc;

use strata_csm_types::{PayloadDest, PayloadIntent};
use strata_db_types::l1_writer::{IntentEntry, IntentStatus, L1BundleStatus};
use strata_primitives::buf::Buf32;
use strata_storage::ops::writer::EnvelopeDataOps;
use tokio::sync::mpsc::Sender;
use tracing::*;

use super::bundler::PendingIntent;

/// A handle to the Envelope task.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have debug impls"
)]
pub struct EnvelopeHandle {
    ops: Arc<EnvelopeDataOps>,
    intent_tx: Sender<PendingIntent>,
}

impl EnvelopeHandle {
    pub fn new(ops: Arc<EnvelopeDataOps>, intent_tx: Sender<PendingIntent>) -> Self {
        Self { ops, intent_tx }
    }

    /// Checks if it is duplicate, if not creates a new [`IntentEntry`] from `intent` and puts it in
    /// the database.
    pub fn submit_intent(&self, intent: PayloadIntent) -> anyhow::Result<()> {
        let id = *intent.commitment();

        // Check if the intent is meant for L1
        if intent.dest() != PayloadDest::L1 {
            warn!(commitment = %id, "Received intent not meant for L1");
            return Ok(());
        }

        debug!(commitment = %id, "Received intent for processing");

        // Check if it is duplicate
        if let Some(existing) = self.ops.get_intent_by_id_blocking(id)? {
            if existing.status != IntentStatus::Abandoned {
                warn!(commitment = %id, "Received duplicate intent");
                return Ok(());
            }
            // Intent indices reference an id-keyed shared entry. Allocating a fresh index below
            // refreshes that entry to Unbundled, so an older index may bundle the retry. Once the
            // shared entry becomes Bundled, every remaining alias is skipped.
        }

        // Create and store IntentEntry
        let entry = IntentEntry::new_unbundled(intent);
        let idx = self.ops.put_intent_entry_blocking(id, entry.clone())?;

        // Send to bundler
        if let Err(e) = self.intent_tx.blocking_send(idx) {
            warn!(%e, %id, "could not send intent entry to bundler");
        }
        Ok(())
    }

    /// Checks if it is duplicate, if not creates a new [`IntentEntry`] from `intent` and puts it in
    /// the database
    pub async fn submit_intent_async(&self, intent: PayloadIntent) -> anyhow::Result<()> {
        self.submit_intent_async_with_idx(intent).await.map(|_| ())
    }

    /// Checks if it is duplicate, if not creates a new [`IntentEntry`] from `intent` and puts it
    /// in the database, returning the intent index in storage.
    pub async fn submit_intent_async_with_idx(
        &self,
        intent: PayloadIntent,
    ) -> anyhow::Result<Option<u64>> {
        let id = *intent.commitment();

        // Check if the intent is meant for L1
        if intent.dest() != PayloadDest::L1 {
            warn!(commitment = %id, "Received intent not meant for L1");
            return Ok(None);
        }

        debug!(commitment = %id, "Received intent for processing");

        // Check if it is duplicate
        if let Some(existing) = self.ops.get_intent_by_id_async(id).await? {
            if existing.status != IntentStatus::Abandoned {
                warn!(commitment = %id, "Received duplicate intent");
                let next_idx = self.ops.get_next_intent_idx_async().await?;
                return self.find_intent_idx_in_range(id, 0, next_idx).await;
            }
            // Intent indices reference an id-keyed shared entry. Allocating a fresh index below
            // refreshes that entry to Unbundled, so older indices for this commitment resolve to
            // the refreshed state too. The bundler may therefore bundle the retry at the older
            // index position, but it creates only one payload: after the shared entry becomes
            // Bundled, every remaining alias is skipped.
        }

        // Create and store IntentEntry
        let entry = IntentEntry::new_unbundled(intent);
        let intent_idx = self.ops.put_intent_entry_async(id, entry.clone()).await?;

        // Send to bundler
        if let Err(e) = self.intent_tx.send(intent_idx).await {
            warn!(%e, %id, "could not send intent entry to bundler");
        }

        Ok(Some(intent_idx))
    }

    async fn find_intent_idx_in_range(
        &self,
        commitment: Buf32,
        start_idx: u64,
        end_idx: u64,
    ) -> anyhow::Result<Option<u64>> {
        for idx in (start_idx..end_idx).rev() {
            let Some(entry) = self.ops.get_intent_by_idx_async(idx).await? else {
                continue;
            };

            if *entry.intent.commitment() == commitment {
                return Ok(Some(idx));
            }
        }

        Ok(None)
    }
}

/// Looks into the database from descending index order till it reaches 0 or `Finalized`
/// [`PayloadEntry`] from which the rest of the [`PayloadEntry`]s should be watched.
pub(crate) fn get_next_payloadidx_to_watch(insc_ops: &EnvelopeDataOps) -> anyhow::Result<u64> {
    let mut next_idx = insc_ops.get_next_payload_idx_blocking()?;

    while next_idx > 0 {
        let Some(payload) = insc_ops.get_payload_entry_by_idx_blocking(next_idx - 1)? else {
            break;
        };
        if payload.status == L1BundleStatus::Finalized {
            break;
        };
        next_idx -= 1;
    }
    Ok(next_idx)
}

#[cfg(test)]
mod test {
    use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
    use strata_db_types::{l1_broadcast::L1TxStatus, l1_writer::BundledPayloadEntry};
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::Buf32;
    use strata_test_utils::ArbitraryGenerator;
    use tokio::sync::mpsc;

    use super::*;
    use crate::writer::{
        bundler::process_unbundled_entries, test_utils::get_envelope_ops,
        watcher::determine_payload_next_status,
    };

    fn test_intent(seed: u8) -> PayloadIntent {
        let payload = L1Payload::new(
            vec![vec![seed; 8]],
            TagData::new(1, seed, vec![]).expect("test tag is valid"),
        )
        .expect("test payload is valid");
        PayloadIntent::new(PayloadDest::L1, Buf32::from([seed; 32]), payload)
    }

    #[test]
    fn test_initialize_writer_state_no_last_payload_idx() {
        let iops = get_envelope_ops();

        let nextidx = iops.get_next_payload_idx_blocking().unwrap();
        assert_eq!(nextidx, 0);

        let idx = get_next_payloadidx_to_watch(&iops).unwrap();

        assert_eq!(idx, 0);
    }

    #[test]
    fn test_initialize_writer_state_with_existing_payloads() {
        let iops = get_envelope_ops();

        let mut e1: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e1.status = L1BundleStatus::Finalized;
        iops.put_payload_entry_blocking(0, e1).unwrap();

        let mut e2: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e2.status = L1BundleStatus::Published;
        iops.put_payload_entry_blocking(1, e2).unwrap();
        let expected_idx = 1; // All entries before this do not need to be watched.

        let mut e3: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e3.status = L1BundleStatus::Unsigned;
        iops.put_payload_entry_blocking(2, e3).unwrap();

        let mut e4: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e4.status = L1BundleStatus::Unsigned;
        iops.put_payload_entry_blocking(3, e4).unwrap();

        let idx = get_next_payloadidx_to_watch(&iops).unwrap();

        assert_eq!(idx, expected_idx);
    }

    #[test]
    fn abandoned_payload_does_not_hide_earlier_unfinalized_entry() {
        let iops = get_envelope_ops();

        let mut finalized: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        finalized.status = L1BundleStatus::Finalized;
        iops.put_payload_entry_blocking(0, finalized).unwrap();

        let mut published: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        published.status = L1BundleStatus::Published;
        iops.put_payload_entry_blocking(1, published).unwrap();

        let mut abandoned: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        abandoned.status = L1BundleStatus::Abandoned;
        iops.put_payload_entry_blocking(2, abandoned).unwrap();

        let mut unsigned: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        unsigned.status = L1BundleStatus::Unsigned;
        iops.put_payload_entry_blocking(3, unsigned).unwrap();

        assert_eq!(get_next_payloadidx_to_watch(&iops).unwrap(), 1);
    }

    #[tokio::test]
    async fn abandoned_intent_allows_identical_payload_resubmission() {
        let ops = get_envelope_ops();
        let (intent_tx, _intent_rx) = mpsc::channel(4);
        let handle = EnvelopeHandle::new(ops.clone(), intent_tx);
        let intent = test_intent(9);
        let intent_id = *intent.commitment();

        let first_idx = handle
            .submit_intent_async_with_idx(intent.clone())
            .await
            .expect("submit first intent")
            .expect("first intent index");
        let mut stored = ops
            .get_intent_by_id_blocking(intent_id)
            .expect("read first intent")
            .expect("first intent exists");
        stored.status = IntentStatus::Abandoned;
        ops.update_intent_entry_blocking(intent_id, stored)
            .expect("abandon first intent");

        let second_idx = handle
            .submit_intent_async_with_idx(intent)
            .await
            .expect("resubmit identical intent")
            .expect("second intent index");
        assert!(second_idx > first_idx);

        process_unbundled_entries(ops.as_ref(), vec![second_idx])
            .await
            .expect("bundle resubmitted intent");
        assert!(matches!(
            ops.get_intent_by_id_blocking(intent_id)
                .expect("read resubmitted intent")
                .expect("resubmitted intent exists")
                .status,
            IntentStatus::Bundled(_)
        ));
    }

    #[tokio::test]
    async fn abandoned_alias_resubmission_bundles_each_commitment_once() {
        let ops = get_envelope_ops();
        let (intent_tx, _intent_rx) = mpsc::channel(4);
        let handle = EnvelopeHandle::new(ops.clone(), intent_tx);
        let intent_x = test_intent(10);
        let payload_x = intent_x.payload().clone();
        let intent_x_id = *intent_x.commitment();
        let intent_y = test_intent(11);
        let payload_y = intent_y.payload().clone();
        let intent_y_id = *intent_y.commitment();

        let first_x_idx = handle
            .submit_intent_async_with_idx(intent_x.clone())
            .await
            .expect("submit first X intent")
            .expect("first X intent index");
        let mut stored_x = ops
            .get_intent_by_id_blocking(intent_x_id)
            .expect("read first X intent")
            .expect("first X intent exists");
        stored_x.status = IntentStatus::Abandoned;
        ops.update_intent_entry_blocking(intent_x_id, stored_x)
            .expect("abandon first X intent");

        let intent_y_idx = handle
            .submit_intent_async_with_idx(intent_y)
            .await
            .expect("submit Y intent")
            .expect("Y intent index");
        let retried_x_idx = handle
            .submit_intent_async_with_idx(intent_x)
            .await
            .expect("resubmit X intent")
            .expect("retried X intent index");

        assert_eq!((first_x_idx, intent_y_idx, retried_x_idx), (0, 1, 2));

        process_unbundled_entries(ops.as_ref(), vec![intent_y_idx, retried_x_idx])
            .await
            .expect("bundle pending intents");

        let stored_x = ops
            .get_intent_by_id_blocking(intent_x_id)
            .expect("read bundled X intent")
            .expect("bundled X intent exists");
        let stored_y = ops
            .get_intent_by_id_blocking(intent_y_id)
            .expect("read bundled Y intent")
            .expect("bundled Y intent exists");
        assert_eq!(stored_x.status, IntentStatus::Bundled(0));
        assert_eq!(stored_y.status, IntentStatus::Bundled(1));
        assert_eq!(
            ops.get_intent_by_idx_blocking(retried_x_idx)
                .expect("read retried X alias")
                .expect("retried X alias exists")
                .status,
            IntentStatus::Bundled(0)
        );

        let next_payload_idx = ops
            .get_next_payload_idx_blocking()
            .expect("read payload count");
        assert_eq!(next_payload_idx, 2);
        let payloads = (0..next_payload_idx)
            .map(|idx| {
                ops.get_payload_entry_by_idx_blocking(idx)
                    .expect("read payload entry")
                    .expect("payload entry exists")
                    .payload
            })
            .collect::<Vec<_>>();
        assert_eq!(
            payloads
                .iter()
                .filter(|payload| **payload == payload_x)
                .count(),
            1
        );
        assert_eq!(
            payloads
                .iter()
                .filter(|payload| **payload == payload_y)
                .count(),
            1
        );
    }

    #[test]
    fn test_determine_payload_next_status() {
        // When both are unpublished
        let (commit_status, reveal_status) = (L1TxStatus::Unpublished, L1TxStatus::Unpublished);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Unpublished);

        // When both are Finalized
        let fin = L1TxStatus::Finalized {
            confirmations: 5,
            block_hash: Buf32::zero(),
            block_height: 100,
        };
        let (commit_status, reveal_status) = (fin.clone(), fin);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Finalized);

        // When both are Confirmed
        let conf = L1TxStatus::Confirmed {
            confirmations: 5,
            block_hash: Buf32::zero(),
            block_height: 100,
        };
        let (commit_status, reveal_status) = (conf.clone(), conf.clone());
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Confirmed);

        // When both are Published
        let publ = L1TxStatus::Published;
        let (commit_status, reveal_status) = (publ.clone(), publ.clone());
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Published);

        // When both have invalid
        let (commit_status, reveal_status) = (L1TxStatus::InvalidInputs, L1TxStatus::InvalidInputs);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::NeedsResign);

        // When reveal has invalid inputs but commit is confirmed. I doubt this would happen in
        // practice for our case.
        // Then the payload status should be NeedsResign i.e. the payload should be signed again and
        // published.
        let (commit_status, reveal_status) = (conf.clone(), L1TxStatus::InvalidInputs);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::NeedsResign);
    }
}
