use std::collections::BTreeSet;

use anyhow::bail;
use strata_db_types::l1_writer::{BundledPayloadEntry, IntentStatus};
use strata_storage::ops::writer::EnvelopeDataOps;
use tracing::*;

pub type PendingIntent = u64;

/// Processes and bundles a list of pending intents into payload entries. Returns a vector of
/// entries which remain unbundled for some reason.
/// The reason could be the entries is too small in size to be included in an envelope and thus
/// makes sense to include once a bunch of entries are collected.
///
/// Ensures previous intents are bundled before bundling a new one.
pub(crate) async fn process_unbundled_entries(
    ops: &EnvelopeDataOps,
    unbundled: Vec<PendingIntent>,
) -> anyhow::Result<Vec<PendingIntent>> {
    let mut pending: BTreeSet<u64> = unbundled.into_iter().collect();

    while let Some(&intent_idx) = pending.first() {
        if !is_predecessor_bundled(ops, intent_idx).await? {
            pending.insert(intent_idx - 1); // intent_idx - 1 is safe here as 0 is already checked
            continue;
        }

        bundle_unbundled_intent(ops, intent_idx).await?;
        pending.remove(&intent_idx);
    }
    // Return empty Vec because each entry is being bundled right now. This might be different in
    // future.
    Ok(vec![])
}

async fn is_predecessor_bundled(ops: &EnvelopeDataOps, idx: u64) -> anyhow::Result<bool> {
    if idx == 0 {
        return Ok(true);
    }

    let prev_idx = idx - 1;
    let Some(prev_entry) = ops.get_intent_by_idx_async(prev_idx).await? else {
        bail!(
            "inconsistent L1 writer DB: missing predecessor intent idx {prev_idx} before bundling idx {idx}"
        );
    };

    match prev_entry.status {
        IntentStatus::Bundled(_) | IntentStatus::Abandoned => Ok(true),
        IntentStatus::Unbundled => Ok(false),
    }
}

async fn bundle_unbundled_intent(ops: &EnvelopeDataOps, intent_idx: u64) -> anyhow::Result<()> {
    let Some(entry) = ops.get_intent_by_idx_async(intent_idx).await? else {
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
    let payload_idx = ops
        .bundle_intent_payload_async(intent_commitment, entry, payload_entry)
        .await?;
    info!(
        %intent_commitment,
        intent_idx,
        payload_idx,
        "bundled L1 intent into payload entry"
    );

    Ok(())
}

/// Retrieves all unbundled intents in ascending index order.
///
/// Intent indices reference id-keyed shared entries, so resubmitting an abandoned intent creates
/// aliases whose statuses change together. A bundled alias at a later index therefore does not
/// prove that every earlier index has been handled. Since this scan runs only during startup, it
/// traverses the complete index range and skips both bundled and abandoned entries.
pub(crate) fn get_initial_unbundled_entries(
    ops: &EnvelopeDataOps,
) -> anyhow::Result<Vec<PendingIntent>> {
    let next_intent_idx = ops.get_next_intent_idx_blocking()?;
    let mut unbundled = Vec::new();

    for intent_idx in 0..next_intent_idx {
        if let Some(intent) = ops.get_intent_by_idx_blocking(intent_idx)? {
            match intent.status {
                IntentStatus::Unbundled => unbundled.push(intent_idx),
                IntentStatus::Bundled(_) | IntentStatus::Abandoned => {}
            }
        } else {
            warn!(%intent_idx, "Could not find expected intent in db");
        }
    }

    Ok(unbundled)
}

#[cfg(test)]
mod tests {
    use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
    use strata_db_types::l1_writer::{BundledPayloadEntry, IntentEntry, IntentStatus};
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::writer::test_utils::get_envelope_ops;

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

        process_unbundled_entries(ops.as_ref(), vec![second_idx])
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
        assert_eq!(stored_first.status, IntentStatus::Bundled(0));
        assert_eq!(stored_second.status, IntentStatus::Bundled(1));
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

        let err = process_unbundled_entries(ops.as_ref(), vec![second_idx])
            .await
            .expect_err("missing predecessor should be fatal");

        assert_eq!(first_idx, 0);
        assert!(
            err.to_string()
                .contains("inconsistent L1 writer DB: missing predecessor intent idx 0"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn abandoned_predecessor_allows_later_intent_to_bundle() {
        let ops = get_envelope_ops();
        let (first_idx, mut first_entry) = put_unbundled_intent(&ops, 1);
        let first_id = *first_entry.intent.commitment();
        first_entry.status = IntentStatus::Abandoned;
        ops.update_intent_entry_blocking(first_id, first_entry)
            .expect("test: abandon first intent");
        let (second_idx, _) = put_unbundled_intent(&ops, 2);

        process_unbundled_entries(ops.as_ref(), vec![second_idx])
            .await
            .expect("test: process pending intent");

        assert_eq!(first_idx, 0);
        assert_eq!(
            ops.get_intent_by_idx_blocking(second_idx)
                .expect("test: read second intent")
                .expect("test: second intent exists")
                .status,
            IntentStatus::Bundled(0)
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

    #[test]
    fn startup_scan_skips_oldest_abandoned_intent() {
        let ops = get_envelope_ops();
        let (_, mut first_entry) = put_unbundled_intent(&ops, 1);
        let first_id = *first_entry.intent.commitment();
        first_entry.status = IntentStatus::Abandoned;
        ops.update_intent_entry_blocking(first_id, first_entry)
            .expect("test: abandon first intent");
        let (second_idx, _) = put_unbundled_intent(&ops, 2);

        assert_eq!(
            get_initial_unbundled_entries(ops.as_ref()).expect("test: scan unbundled"),
            vec![second_idx]
        );
    }

    #[test]
    fn startup_scan_recovers_unbundled_intent_before_bundled_alias() {
        let ops = get_envelope_ops();
        let (first_a_idx, mut first_a_entry) = put_unbundled_intent(&ops, 20);
        let intent_a_id = *first_a_entry.intent.commitment();
        first_a_entry.status = IntentStatus::Abandoned;
        ops.update_intent_entry_blocking(intent_a_id, first_a_entry)
            .expect("test: abandon first A intent");

        let (intent_b_idx, _) = put_unbundled_intent(&ops, 21);
        let (retried_a_idx, retried_a_entry) = put_unbundled_intent(&ops, 20);
        assert_eq!((first_a_idx, intent_b_idx, retried_a_idx), (0, 1, 2));

        let retried_a_payload =
            BundledPayloadEntry::new_unsigned(retried_a_entry.payload().clone());
        ops.bundle_intent_payload_blocking(intent_a_id, retried_a_entry, retried_a_payload)
            .expect("test: bundle only retried A before simulated crash");

        assert!(matches!(
            ops.get_intent_by_idx_blocking(retried_a_idx)
                .expect("test: read retried A alias")
                .expect("test: retried A alias exists")
                .status,
            IntentStatus::Bundled(_)
        ));
        assert_eq!(
            get_initial_unbundled_entries(ops.as_ref())
                .expect("test: scan through bundled alias after restart"),
            vec![intent_b_idx]
        );
    }

    #[tokio::test]
    async fn startup_scan_skips_abandoned_and_recovers_earliest_unbundled_intent() {
        let ops = get_envelope_ops();
        let (first_non_checkpoint_idx, _) = put_unbundled_intent(&ops, 10);
        let (abandoned_idx, mut abandoned_entry) = put_unbundled_intent(&ops, 11);
        let abandoned_id = *abandoned_entry.intent.commitment();
        abandoned_entry.status = IntentStatus::Abandoned;
        ops.update_intent_entry_blocking(abandoned_id, abandoned_entry)
            .expect("test: abandon middle intent");
        let (last_idx, _) = put_unbundled_intent(&ops, 12);

        let unbundled = get_initial_unbundled_entries(ops.as_ref())
            .expect("test: scan across abandoned intent");
        assert_eq!(unbundled, vec![first_non_checkpoint_idx, last_idx]);

        process_unbundled_entries(ops.as_ref(), unbundled)
            .await
            .expect("test: bundle recovered intents");

        assert_eq!(
            ops.get_intent_by_idx_blocking(first_non_checkpoint_idx)
                .expect("test: read first intent")
                .expect("test: first intent exists")
                .status,
            IntentStatus::Bundled(0)
        );
        assert_eq!(
            ops.get_intent_by_idx_blocking(abandoned_idx)
                .expect("test: read abandoned intent")
                .expect("test: abandoned intent exists")
                .status,
            IntentStatus::Abandoned
        );
        assert_eq!(
            ops.get_intent_by_idx_blocking(last_idx)
                .expect("test: read last intent")
                .expect("test: last intent exists")
                .status,
            IntentStatus::Bundled(1)
        );
    }
}
