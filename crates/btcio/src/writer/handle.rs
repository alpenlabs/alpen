use std::sync::Arc;

use strata_csm_types::{PayloadDest, PayloadIntent};
use strata_db_types::types::{IntentEntry, L1BundleStatus};
use strata_primitives::buf::Buf32;
use strata_storage::ops::writer::EnvelopeDataOps;
use tokio::sync::mpsc::Sender;
use tracing::*;

/// A handle to the Envelope task.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have debug impls"
)]
pub struct EnvelopeHandle {
    ops: Arc<EnvelopeDataOps>,
    intent_tx: Sender<IntentEntry>,
}

impl EnvelopeHandle {
    pub fn new(ops: Arc<EnvelopeDataOps>, intent_tx: Sender<IntentEntry>) -> Self {
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
        if self.ops.get_intent_by_id_blocking(id)?.is_some() {
            warn!(commitment = %id, "Received duplicate intent");
            return Ok(());
        }

        // Create and store IntentEntry
        let entry = IntentEntry::new_unbundled(intent);
        let _idx = self.ops.put_intent_entry_blocking(id, entry.clone())?;

        // Send to bundler
        if let Err(e) = self.intent_tx.blocking_send(entry) {
            warn!("Could not send intent entry to bundler: {:?}", e);
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
        if self.ops.get_intent_by_id_async(id).await?.is_some() {
            warn!(commitment = %id, "Received duplicate intent");
            let next_idx = self.ops.get_next_intent_idx_async().await?;
            return self.find_intent_idx_in_range(id, 0, next_idx).await;
        }

        // Create and store IntentEntry
        let entry = IntentEntry::new_unbundled(intent);
        let intent_idx = self.ops.put_intent_entry_async(id, entry.clone()).await?;

        // Send to bundler
        if let Err(e) = self.intent_tx.send(entry).await {
            warn!("Could not send intent entry to bundler: {:?}", e);
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
    use strata_db_types::types::{BundledPayloadEntry, L1TxStatus};
    use strata_primitives::buf::Buf32;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::writer::{
        test_utils::get_envelope_ops, watcher_service::determine_payload_next_status,
    };

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
