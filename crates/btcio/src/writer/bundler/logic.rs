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
        IntentStatus::Bundled(_) => Ok(true),
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
                IntentStatus::Bundled(_) => {
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
}
