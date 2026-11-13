//! Mempool service implementation.

use std::marker::PhantomData;

use serde::Serialize;
use strata_ol_state_provider::StateProvider;
use strata_service::{AsyncService, Response, Service};
use tracing::{debug, info};

use crate::builder::MempoolInputMessage;
use crate::state::MempoolServiceState;
use crate::types::OLMempoolStats;
use crate::{MempoolCommand, MempoolTip};

/// Service status for mempool.
#[derive(Debug, Clone, Serialize)]
pub struct MempoolServiceStatus {
    pub stats: OLMempoolStats,
}

/// Mempool service that processes commands.
///
/// # Type Parameters
///
/// - `P`: The state provider type that implements [`StateProvider`].
#[derive(Debug)]
pub(crate) struct MempoolService<P: StateProvider> {
    _phantom: PhantomData<P>,
}

impl<P: StateProvider> Service for MempoolService<P> {
    type State = MempoolServiceState<P>;
    type Msg = MempoolInputMessage;
    type Status = MempoolServiceStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        MempoolServiceStatus {
            stats: state.stats().clone(),
        }
    }
}

impl<P: StateProvider> AsyncService for MempoolService<P> {
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        info!(
            mempool_size = %state.stats().mempool_size(),
            "ol mempool service launched"
        );
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match input {
            MempoolInputMessage::Command(cmd) => match cmd {
                MempoolCommand::SubmitTransaction { tx, completion } => {
                    let result = state.handle_submit_transaction(tx).await;
                    completion.send(result).await;
                }

                MempoolCommand::GetCandidates { completion } => {
                    completion.send(state.handle_get_candidates()).await;
                }

                MempoolCommand::ReportInvalidTransactions { txs, completion } => {
                    debug!(count = %txs.len(), "reporting invalid transactions");
                    state.handle_report_invalid_transactions(txs);
                    completion.send(()).await;
                }
            },

            MempoolInputMessage::ChainUpdate(update) => {
                let new_tip = MempoolTip::from_sync_status(&update.new_status());
                // A failed update leaves the mempool out of sync at the old
                // tip, which the state logs, and the next update retries it.
                // Stopping the service would leave the node without a mempool
                // instead.
                let _ = state.handle_chain_update(new_tip).await;
            }
        }

        Ok(Response::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_identifiers::{EpochCommitment, OLTxId};
    use strata_service::CommandCompletionSender;
    use strata_status::{OLSyncStatus, OLSyncStatusUpdate};
    use tokio::sync::oneshot;

    use super::*;
    use crate::test_utils::{
        InMemoryStateProvider, create_test_block_commitment, create_test_context,
        create_test_snark_tx_with_seq_no, create_test_state_provider,
    };
    use crate::types::OLMempoolConfig;
    use crate::{MempoolCandidates, MempoolTxInvalidReason, OLMempoolError, OLMempoolResult};

    #[tokio::test]
    async fn test_service_submit_transaction() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));

        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let expected_txid = tx.compute_txid();

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::SubmitTransaction {
            tx: Box::new(tx),
            completion,
        };

        MempoolService::process_input(&mut state, MempoolInputMessage::Command(command))
            .await
            .expect("Should process command");

        let result: OLMempoolResult<OLTxId> = rx.await.expect("Should receive result");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected_txid);
    }

    #[tokio::test]
    async fn test_service_get_candidates_preserves_order_and_bodies() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(OLMempoolConfig::default(), provider));
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(1, 1);
        for tx in [&tx1, &tx2] {
            state
                .handle_submit_transaction(Box::new(tx.clone()))
                .await
                .expect("submit transaction");
        }

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);
        let command = MempoolCommand::GetCandidates { completion };
        MempoolService::process_input(&mut state, MempoolInputMessage::Command(command))
            .await
            .expect("fetch candidates");

        let candidates: MempoolCandidates = rx.await.expect("receive candidates");
        let candidates = candidates.collect::<Vec<_>>();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].txid(), tx1.compute_txid());
        assert_eq!(candidates[0].transaction(), &tx1);
        assert_eq!(candidates[1].txid(), tx2.compute_txid());
        assert_eq!(candidates[1].transaction(), &tx2);
    }

    #[tokio::test]
    async fn test_service_report_invalid_transactions() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));

        let mut state =
            MempoolServiceState::new_with_context(context.clone(), MempoolTip::new(tip, None))
                .await
                .unwrap();

        // Account 1: tx1 (seq 0), tx2 (seq 1) - tx2 cascades when tx1 removed
        // Account 2: tx3 (seq 0) - independent
        let txs: [(u8, u64); 3] = [(1, 0), (1, 1), (2, 0)];
        let mut txids = Vec::new();
        for (account, seq_no) in txs {
            let tx = create_test_snark_tx_with_seq_no(account, seq_no);
            txids.push(tx.compute_txid());
            state.handle_submit_transaction(Box::new(tx)).await.unwrap();
        }
        let [txid1, txid2, txid3] = txids.try_into().unwrap();

        for txid in [txid1, txid2, txid3] {
            assert!(state.contains(&txid));
        }

        // Report tx1 as Invalid (should be removed)
        // Report tx3 as Failed (should NOT be removed)
        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::ReportInvalidTransactions {
            txs: vec![
                (txid1, MempoolTxInvalidReason::Invalid),
                (txid3, MempoolTxInvalidReason::Failed),
            ],
            completion,
        };

        MempoolService::process_input(&mut state, MempoolInputMessage::Command(command))
            .await
            .expect("Should process command");

        let _: () = rx.await.expect("Should receive result");

        // Verify tx1 removed (Invalid), tx2 cascade removed, tx3 remains (Failed)
        assert!(!state.contains(&txid1), "Invalid should remove tx1");
        assert!(!state.contains(&txid2), "Invalid should cascade remove tx2");
        assert!(state.contains(&txid3), "Failed should NOT remove tx3");
    }

    #[tokio::test]
    async fn test_service_stats() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));
        let mut state =
            MempoolServiceState::new_with_context(context.clone(), MempoolTip::new(tip, None))
                .await
                .unwrap();

        // Add a transaction via handle_submit_transaction
        let tx = create_test_snark_tx_with_seq_no(1, 0);
        state
            .handle_submit_transaction(Box::new(tx))
            .await
            .expect("Should add tx");

        // Get stats via Service::get_status (not command)
        let status = MempoolService::get_status(&state);
        assert_eq!(status.stats.mempool_size(), 1);
        assert_eq!(status.stats.enqueues_accepted(), 1);
    }

    /// A chain update the mempool cannot apply, here one whose block storage
    /// lacks, leaves the service running but out of sync: it serves no
    /// candidates and rejects submissions. An update that succeeds, here a
    /// repeat of the mempool's own tip, brings it back.
    #[tokio::test]
    async fn test_service_survives_a_failed_chain_update() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(OLMempoolConfig::default(), provider));
        let genesis_l1_block = context.genesis_l1_block;
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();
        let held = create_test_snark_tx_with_seq_no(1, 0);
        state
            .handle_submit_transaction(Box::new(held.clone()))
            .await
            .unwrap();
        let chain_update = |tip| {
            let status = OLSyncStatus::new(
                tip,
                0,
                false,
                EpochCommitment::null(),
                EpochCommitment::null(),
                EpochCommitment::null(),
                genesis_l1_block,
            );
            MempoolInputMessage::ChainUpdate(OLSyncStatusUpdate::new(status))
        };

        let response = MempoolService::process_input(
            &mut state,
            chain_update(create_test_block_commitment(101)),
        )
        .await
        .expect("a failed chain update does not stop the service");
        assert!(matches!(response, Response::Continue));
        assert_eq!(get_candidate_txids(&mut state).await, Vec::<OLTxId>::new());
        let err = state
            .handle_submit_transaction(Box::new(create_test_snark_tx_with_seq_no(2, 0)))
            .await
            .expect_err("rejected while out of sync");
        assert!(matches!(err, OLMempoolError::OutOfSync), "{err}");

        MempoolService::process_input(&mut state, chain_update(tip))
            .await
            .unwrap();
        assert_eq!(
            get_candidate_txids(&mut state).await,
            vec![held.compute_txid()]
        );
        state
            .handle_submit_transaction(Box::new(create_test_snark_tx_with_seq_no(2, 0)))
            .await
            .expect("admission resumes");
    }

    /// Fetches the candidates through the service.
    async fn get_candidate_txids(
        state: &mut MempoolServiceState<InMemoryStateProvider>,
    ) -> Vec<OLTxId> {
        let (tx_sender, rx) = oneshot::channel();
        let command = MempoolCommand::GetCandidates {
            completion: CommandCompletionSender::new(tx_sender),
        };
        MempoolService::process_input(state, MempoolInputMessage::Command(command))
            .await
            .unwrap();
        let candidates: MempoolCandidates = rx.await.unwrap();
        candidates.map(|candidate| candidate.txid()).collect()
    }
}
