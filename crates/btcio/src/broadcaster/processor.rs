use bitcoin::Txid;
use strata_db_types::types::{L1TxEntry, L1TxStatus};
use tracing::*;

use super::{
    error::{BroadcasterError, BroadcasterResult},
    io::{BroadcasterIoContext, PublishTxOutcome},
    state::{BroadcasterState, IndexedEntry},
};
use crate::BtcioParams;

/// Processes unfinalized entries and returns the indexed entries whose status changed.
pub(super) async fn process_unfinalized_entries<C>(
    unfinalized_entries: impl Iterator<Item = &IndexedEntry>,
    io: &C,
    params: &BtcioParams,
) -> BroadcasterResult<Vec<IndexedEntry>>
where
    C: BroadcasterIoContext,
{
    let mut updated_entries = Vec::new();

    for entry in unfinalized_entries {
        let idx = *entry.index();
        let txentry = entry.item();
        let txid = txentry
            .try_to_tx()
            .map_err(|e| BroadcasterError::Other(e.to_string()))?
            .compute_txid();

        let updated_status = process_tx_entry(io, txentry, &txid, params).await?;

        if let Some(status) = updated_status {
            let mut new_txentry = txentry.clone();
            new_txentry.status = status;
            updated_entries.push(IndexedEntry::new(idx, new_txentry));
        }
    }

    Ok(updated_entries)
}

/// Computes the next status for a single entry, or `None` when no update is needed.
#[instrument(
    skip_all,
    fields(component = "btcio_broadcaster", %txid),
    name = "process_txentry"
)]
pub(super) async fn process_tx_entry<C>(
    io: &C,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &BtcioParams,
) -> BroadcasterResult<Option<L1TxStatus>>
where
    C: BroadcasterIoContext,
{
    let result = match txentry.status {
        L1TxStatus::Unpublished => publish_tx(io, txentry).await.map(Some),
        L1TxStatus::Published | L1TxStatus::Confirmed { .. } => {
            check_tx_confirmations(io, txentry, txid, params)
                .await
                .map(Some)
        }
        L1TxStatus::Finalized { .. } => Ok(None),
        L1TxStatus::InvalidInputs => Ok(None),
    };
    if let Ok(ref updated_status) = result {
        debug!(?updated_status);
    }
    result
}

/// Resolves `Published`/`Confirmed` entries to their next confirmation-derived status.
async fn check_tx_confirmations<C>(
    io: &C,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &BtcioParams,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    async {
        let txinfo_res = io.get_transaction(txid).await;
        debug!(?txinfo_res, "checked transaction status");

        let reorg_safe_depth = params.l1_reorg_safe_depth();
        let reorg_safe_depth: i64 = reorg_safe_depth.into();

        match txinfo_res {
            Ok(Some(info)) => match (info.confirmations, &txentry.status) {
                // A previously published tx with 0 confirmations remains published.
                (0, L1TxStatus::Published) => Ok(L1TxStatus::Published),
                // A previously confirmed tx with 0 confirmations regresses to unpublished.
                (0, _) => Ok(L1TxStatus::Unpublished),
                (confirmations, _) => {
                    let block_hash = info.block_hash.expect("confirmed tx must have block_hash");
                    let block_height = info
                        .block_height
                        .expect("confirmed tx must have block_height");

                    if confirmations >= reorg_safe_depth {
                        Ok(L1TxStatus::Finalized {
                            confirmations: confirmations as u64,
                            block_hash,
                            block_height,
                        })
                    } else {
                        Ok(L1TxStatus::Confirmed {
                            confirmations: confirmations as u64,
                            block_hash,
                            block_height,
                        })
                    }
                }
            },
            Ok(None) => Ok(L1TxStatus::Unpublished),
            Err(e) => Err(e),
        }
    }
    .instrument(debug_span!(
        "check_tx_confirmations",
        component = "btcio_broadcaster",
        %txid,
        current_status = ?txentry.status
    ))
    .await
}

/// Attempts to broadcast an unpublished entry and maps publication outcomes to statuses.
async fn publish_tx<C>(io: &C, txentry: &L1TxEntry) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    let tx = txentry.try_to_tx().expect("could not deserialize tx");
    let txid = tx.compute_txid();
    let input_count = tx.input.len();
    let output_count = tx.output.len();

    async {
        if tx.input.is_empty() {
            error!("tx has no inputs, excluding from broadcast");
            return Ok(L1TxStatus::InvalidInputs);
        }

        debug!("publishing tx");
        match io.send_raw_transaction(&tx).await {
            Ok(PublishTxOutcome::Published) => Ok(L1TxStatus::Published),
            Ok(PublishTxOutcome::AlreadyInMempool) => Ok(L1TxStatus::Published),
            Ok(PublishTxOutcome::InvalidInputs) => {
                warn!("tx excluded due to invalid inputs");
                Ok(L1TxStatus::InvalidInputs)
            }
            Ok(PublishTxOutcome::RetryLater { reason }) => {
                warn!(%reason, "broadcast should be retried on next poll");
                Ok(L1TxStatus::Unpublished)
            }
            Err(err) => {
                warn!(?err, "errored while broadcasting");
                Err(err)
            }
        }
    }
    .instrument(debug_span!(
        "publish_tx",
        component = "btcio_broadcaster",
        %txid,
        input_count,
        output_count,
        current_status = ?txentry.status
    ))
    .await
}

/// Updates state by folding updated entries and newly seen unfinalized entries from IO context.
pub(super) async fn update_state<C>(
    state: &mut BroadcasterState,
    updated_entries: impl Iterator<Item = IndexedEntry>,
    io: &C,
) -> BroadcasterResult<()>
where
    C: BroadcasterIoContext,
{
    let unfinalized_entries: Vec<_> = updated_entries
        .filter(|entry| !entry.item().is_finalized() && entry.item().is_valid())
        .collect();

    let next_idx = io.get_next_tx_idx().await?;
    if next_idx < state.next_idx {
        return Err(BroadcasterError::InconsistentNextIdx {
            expected: state.next_idx,
            got: next_idx,
        });
    }

    let new_unfinalized_entries = fetch_unfinalized_entries(io, state.next_idx, next_idx).await?;

    state.unfinalized_entries = unfinalized_entries;
    state.unfinalized_entries.extend(new_unfinalized_entries);
    state.next_idx = next_idx;
    Ok(())
}

/// Returns unfinalized but valid [`L1TxEntry`]s from context-backed DB starting from index `from`
/// until `to` non-inclusive.
pub(super) async fn fetch_unfinalized_entries<C>(
    io: &C,
    from: u64,
    to: u64,
) -> BroadcasterResult<Vec<IndexedEntry>>
where
    C: BroadcasterIoContext,
{
    let mut unfinalized_entries = Vec::new();
    for idx in from..to {
        let Some(txentry) = io.get_tx_entry(idx).await? else {
            break;
        };

        if !txentry.is_valid() {
            error!(%idx, status = ?txentry.status, "invalid broadcaster entry in DB; skipping");
            continue;
        }

        if txentry.is_finalized() {
            continue;
        }

        unfinalized_entries.push(IndexedEntry::new(idx, txentry));
    }
    Ok(unfinalized_entries)
}

#[cfg(test)]
mod test {
    use std::{collections::BTreeMap, future::Future};

    use bitcoin::{Transaction, Txid};
    use proptest::prelude::*;
    use strata_db_types::types::{L1TxEntry, L1TxStatus};
    use strata_identifiers::test_utils::buf32_strategy;
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::{buf::Buf32, L1Height};
    use tokio::runtime::Builder;

    use super::*;
    use crate::{
        broadcaster::io::{BroadcasterIoContext, PublishTxOutcome, TxConfirmationInfo},
        test_utils::gen_l1_tx_entry_with_status,
    };

    const TEST_REORG_DEPTH: u32 = 6;
    const TEST_GENESIS_L1_HEIGHT: L1Height = 0;

    #[derive(Clone, Debug)]
    enum MockTxLookupResult {
        Missing,
        Found(TxConfirmationInfo),
    }

    #[derive(Clone, Debug)]
    enum MockBroadcastResult {
        Published,
        AlreadyInMempool,
        InvalidInputs,
        RetryLater,
    }

    #[derive(Clone, Debug, Default)]
    struct MockIoContext {
        next_idx: u64,
        entries: BTreeMap<u64, L1TxEntry>,
        tx_lookup: BTreeMap<Txid, MockTxLookupResult>,
        broadcast_results: BTreeMap<Txid, MockBroadcastResult>,
    }

    impl MockIoContext {
        fn with_tx_lookup(mut self, txid: Txid, result: MockTxLookupResult) -> Self {
            self.tx_lookup.insert(txid, result);
            self
        }

        fn with_broadcast_result(mut self, txid: Txid, result: MockBroadcastResult) -> Self {
            self.broadcast_results.insert(txid, result);
            self
        }
    }

    impl BroadcasterIoContext for MockIoContext {
        async fn get_next_tx_idx(&self) -> BroadcasterResult<u64> {
            Ok(self.next_idx)
        }

        async fn get_tx_entry(&self, idx: u64) -> BroadcasterResult<Option<L1TxEntry>> {
            Ok(self.entries.get(&idx).cloned())
        }

        async fn put_tx_entry_by_idx(&self, _idx: u64, _entry: L1TxEntry) -> BroadcasterResult<()> {
            Ok(())
        }

        async fn get_transaction<'a>(
            &'a self,
            txid: &'a Txid,
        ) -> BroadcasterResult<Option<TxConfirmationInfo>> {
            let result = self
                .tx_lookup
                .get(txid)
                .cloned()
                .unwrap_or(MockTxLookupResult::Missing);

            match result {
                MockTxLookupResult::Missing => Ok(None),
                MockTxLookupResult::Found(info) => Ok(Some(info)),
            }
        }

        async fn send_raw_transaction<'a>(
            &'a self,
            tx: &'a Transaction,
        ) -> BroadcasterResult<PublishTxOutcome> {
            let txid = tx.compute_txid();
            let result = self
                .broadcast_results
                .get(&txid)
                .cloned()
                .unwrap_or(MockBroadcastResult::Published);

            match result {
                MockBroadcastResult::Published => Ok(PublishTxOutcome::Published),
                MockBroadcastResult::AlreadyInMempool => Ok(PublishTxOutcome::AlreadyInMempool),
                MockBroadcastResult::InvalidInputs => Ok(PublishTxOutcome::InvalidInputs),
                MockBroadcastResult::RetryLater => Ok(PublishTxOutcome::RetryLater {
                    reason: "mock retry".into(),
                }),
            }
        }
    }

    fn get_test_btcio_params() -> BtcioParams {
        BtcioParams::new(
            TEST_REORG_DEPTH,          // l1_reorg_safe_depth
            MagicBytes::new(*b"ALPN"), // magic_bytes
            TEST_GENESIS_L1_HEIGHT,    // genesis_l1_height
        )
    }

    fn entry_with_txid(status: L1TxStatus) -> (L1TxEntry, Txid) {
        let entry = gen_l1_tx_entry_with_status(status);
        let txid = entry.try_to_tx().unwrap().compute_txid();
        (entry, txid)
    }

    fn confirmation_info(
        confirmations: i64,
        block_height: L1Height,
        block_hash: Buf32,
    ) -> TxConfirmationInfo {
        if confirmations == 0 {
            TxConfirmationInfo {
                confirmations,
                block_hash: None,
                block_height: None,
            }
        } else {
            TxConfirmationInfo {
                confirmations,
                block_hash: Some(block_hash),
                block_height: Some(block_height),
            }
        }
    }

    fn status_with_confirmations(
        confirmations: u64,
        block_height: L1Height,
        block_hash: Buf32,
        finalized: bool,
    ) -> L1TxStatus {
        if finalized {
            L1TxStatus::Finalized {
                confirmations,
                block_hash,
                block_height,
            }
        } else {
            L1TxStatus::Confirmed {
                confirmations,
                block_hash,
                block_height,
            }
        }
    }

    fn confirmed_status(
        confirmations: u64,
        block_height: L1Height,
        block_hash: Buf32,
    ) -> L1TxStatus {
        status_with_confirmations(confirmations, block_height, block_hash, false)
    }

    fn finalized_status(
        confirmations: u64,
        block_height: L1Height,
        block_hash: Buf32,
    ) -> L1TxStatus {
        status_with_confirmations(confirmations, block_height, block_hash, true)
    }

    async fn process_status(
        io: &MockIoContext,
        entry: &L1TxEntry,
        txid: &Txid,
        params: &BtcioParams,
    ) -> Option<L1TxStatus> {
        process_tx_entry(io, entry, txid, params).await.unwrap()
    }

    fn run_async_test<F>(future: F)
    where
        F: Future<Output = ()>,
    {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(future);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io =
            MockIoContext::default().with_broadcast_result(txid, MockBroadcastResult::Published);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Status should be published for unpublished tx after successful broadcast"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_status_500_keeps_unpublished() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io =
            MockIoContext::default().with_broadcast_result(txid, MockBroadcastResult::RetryLater);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Unpublished),
            "HTTP 500 send_raw_transaction errors should keep tx unpublished for retry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_server_minus22_marks_invalid_inputs() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_broadcast_result(txid, MockBroadcastResult::InvalidInputs);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::InvalidInputs),
            "Server(-22, ..) send_raw_transaction errors should mark tx invalid"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_already_in_mempool_marks_published() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_broadcast_result(txid, MockBroadcastResult::AlreadyInMempool);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Server(-25, ..) send_raw_transaction should be treated as already published"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]

        #[test]
        fn test_handle_published_entry(
            block_height in 1_u32..1_000_000,
            block_hash in buf32_strategy(),
        ) {
            run_async_test(async move {
                let (e, txid) = entry_with_txid(L1TxStatus::Published);
                let btcio_params = get_test_btcio_params();
                let reorg_depth = i64::from(btcio_params.l1_reorg_safe_depth());

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(0, block_height, block_hash)),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(L1TxStatus::Published),
                    "Status should not change if no confirmations for a published tx"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth - 1,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(confirmed_status(
                        (reorg_depth - 1) as u64,
                        block_height,
                        block_hash,
                    )),
                    "Status should be confirmed if 0 < confirmations < finality_depth"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(finalized_status(reorg_depth as u64, block_height, block_hash)),
                    "Status should be finalized if confirmations >= finality_depth"
                );
            });
        }

        #[test]
        fn test_handle_confirmed_entry(
            block_height in 1_u32..1_000_000,
            block_hash in buf32_strategy(),
        ) {
            run_async_test(async move {
                let (e, txid) = entry_with_txid(confirmed_status(1, block_height, block_hash));
                let btcio_params = get_test_btcio_params();
                let reorg_depth = i64::from(btcio_params.l1_reorg_safe_depth());

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(0, block_height, block_hash)),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(L1TxStatus::Unpublished),
                    "Status should revert to unpublished if confirmed tx now has 0 confirmations"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth - 1,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(confirmed_status(
                        (reorg_depth - 1) as u64,
                        block_height,
                        block_hash,
                    )),
                    "Status should remain confirmed if 0 < confirmations < finality_depth"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(finalized_status(reorg_depth as u64, block_height, block_hash)),
                    "Status should be finalized if confirmations >= finality_depth"
                );
            });
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_finalized_entry() {
        let (e, txid) = entry_with_txid(finalized_status(1, 1, Buf32::zero()));
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default();
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(res, None, "Finalized tx should remain unchanged");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_excluded_entry() {
        let e = gen_l1_tx_entry_with_status(L1TxStatus::InvalidInputs);
        let btcio_params = get_test_btcio_params();
        let txid = e.try_to_tx().unwrap().compute_txid();

        let io = MockIoContext::default();
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(res, None, "InvalidInputs tx should remain unchanged");
    }
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]

        #[test]
        fn test_process_unfinalized_entries(
            seed_idx in 1_u64..1_000_000,
            block_height in 1_u32..1_000_000,
            block_hash in buf32_strategy(),
        ) {
            run_async_test(async move {
                let btcio_params = get_test_btcio_params();
                let reorg_depth = btcio_params.l1_reorg_safe_depth() as u64;

                let (e1, txid1) = entry_with_txid(L1TxStatus::Unpublished);
                let i1 = seed_idx;

                let e2 = gen_l1_tx_entry_with_status(L1TxStatus::InvalidInputs);
                let i2 = seed_idx + 1;

                let (e3, txid3) = entry_with_txid(L1TxStatus::Published);
                let i3 = seed_idx + 2;

                let unfinalized_entries = [
                    IndexedEntry::new(i1, e1),
                    IndexedEntry::new(i2, e2),
                    IndexedEntry::new(i3, e3),
                ];

                let io = MockIoContext::default()
                    .with_broadcast_result(txid1, MockBroadcastResult::Published)
                    .with_tx_lookup(
                        txid3,
                        MockTxLookupResult::Found(confirmation_info(
                            reorg_depth as i64,
                            block_height,
                            block_hash,
                        )),
                    );

                let updated_entries = process_unfinalized_entries(
                    unfinalized_entries.iter(),
                    &io,
                    &btcio_params,
                )
                .await
                .unwrap();

                assert_eq!(
                    updated_entries
                        .iter()
                        .find(|e| *e.index() == i1)
                        .map(|e| e.item().status.clone())
                        .unwrap(),
                    L1TxStatus::Published,
                    "unpublished tx should be published"
                );
                assert_eq!(
                    updated_entries
                        .iter()
                        .find(|e| *e.index() == i3)
                        .map(|e| e.item().status.clone())
                        .unwrap(),
                    finalized_status(reorg_depth, block_height, block_hash),
                    "published tx should be finalized"
                );
            });
        }
    }
}
