//! Mempool service implementation.

use strata_service::{AsyncService, Response, Service};

use crate::{MempoolCommand, state::MempoolServiceState};

/// Mempool service that processes commands.
#[derive(Debug)]
#[cfg_attr(not(test), expect(dead_code, reason = "will be used via builder"))]
pub(crate) struct MempoolService;

impl Service for MempoolService {
    type State = MempoolServiceState;
    type Msg = MempoolCommand;
    type Status = MempoolServiceStatus;

    fn get_status(_state: &Self::State) -> Self::Status {
        MempoolServiceStatus
    }
}

impl AsyncService for MempoolService {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            MempoolCommand::SubmitTransaction {
                tx_bytes,
                completion,
            } => {
                let result = state.handle_submit_transaction(tx_bytes.clone()).await;
                completion.send(result).await;
            }

            MempoolCommand::BestTransactions { completion } => {
                let result = state.handle_best_transactions().await;
                completion.send(result).await;
            }

            MempoolCommand::RemoveTransactions { ids, completion } => {
                let result = state.handle_remove_transactions(ids.clone());
                completion.send(result).await;
            }

            MempoolCommand::Contains { id, completion } => {
                let result = state.contains(id);
                completion.send(result).await;
            }

            MempoolCommand::Stats { completion } => {
                let stats = state.stats();
                completion.send(stats).await;
            }

            MempoolCommand::ChainUpdate {
                new_tip,
                completion,
            } => {
                let result = state.handle_chain_update(*new_tip).await;
                completion.send(result).await;
            }
        }

        Ok(Response::Continue)
    }
}

/// Service status for mempool.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(not(test), expect(dead_code, reason = "will be used via builder"))]
pub(crate) struct MempoolServiceStatus;

#[cfg(test)]
mod tests {
    use strata_identifiers::OLTxId;
    use strata_service::CommandCompletionSender;
    use tokio::sync::oneshot;

    use super::*;
    use crate::{
        OLMempoolResult, OLMempoolTransaction,
        test_utils::{
            create_test_block_commitment, create_test_context_arc_with_state,
            create_test_snark_tx_with_seq_no,
        },
        types::OLMempoolStats,
    };

    #[tokio::test]
    async fn test_service_submit_transaction() {
        let tip = create_test_block_commitment(100);
        let context = create_test_context_arc_with_state(tip).await;
        let mut state = MempoolServiceState::new_with_context(context, tip);

        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let tx_bytes = ssz::Encode::as_ssz_bytes(&tx);
        let expected_txid = tx.compute_txid();

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::SubmitTransaction {
            tx_bytes,
            completion,
        };

        MempoolService::process_input(&mut state, &command)
            .await
            .expect("Should process command");

        let result: OLMempoolResult<OLTxId> = rx.await.expect("Should receive result");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected_txid);
    }

    #[tokio::test]
    async fn test_service_get_transactions() {
        let tip = create_test_block_commitment(100);
        let context = create_test_context_arc_with_state(tip).await;
        let mut state = MempoolServiceState::new_with_context(context.clone(), tip);

        // Add some transactions via handle_submit_transaction
        // Use sequential seq_nos (0, 1) for the same account to pass gap checking
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(1, 1);

        let tx1_bytes = ssz::Encode::as_ssz_bytes(&tx1);
        let tx2_bytes = ssz::Encode::as_ssz_bytes(&tx2);

        state
            .handle_submit_transaction(tx1_bytes)
            .await
            .expect("Should add tx1");
        state
            .handle_submit_transaction(tx2_bytes)
            .await
            .expect("Should add tx2");

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::BestTransactions { completion };

        MempoolService::process_input(&mut state, &command)
            .await
            .expect("Should process command");

        let result: OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>> =
            rx.await.expect("Should receive result");
        assert!(result.is_ok());
        let txs = result.unwrap();
        assert_eq!(txs.len(), 2);
    }

    #[tokio::test]
    async fn test_service_remove_transactions() {
        let tip = create_test_block_commitment(100);
        let context = create_test_context_arc_with_state(tip).await;
        let mut state = MempoolServiceState::new_with_context(context.clone(), tip);

        // Add a transaction via handle_submit_transaction
        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let txid = tx.compute_txid();
        let tx_bytes = ssz::Encode::as_ssz_bytes(&tx);

        state
            .handle_submit_transaction(tx_bytes)
            .await
            .expect("Should add tx");

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::RemoveTransactions {
            ids: vec![txid],
            completion,
        };

        MempoolService::process_input(&mut state, &command)
            .await
            .expect("Should process command");

        let result: OLMempoolResult<Vec<OLTxId>> = rx.await.expect("Should receive result");
        assert!(result.is_ok());
        let removed = result.unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid);

        // Verify transaction is gone
        assert!(!state.contains(&txid));
    }

    #[tokio::test]
    async fn test_service_contains() {
        let tip = create_test_block_commitment(100);
        let context = create_test_context_arc_with_state(tip).await;
        let mut state = MempoolServiceState::new_with_context(context.clone(), tip);

        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let txid = tx.compute_txid();
        let tx_bytes = ssz::Encode::as_ssz_bytes(&tx);

        state
            .handle_submit_transaction(tx_bytes)
            .await
            .expect("Should add tx");

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::Contains {
            id: txid,
            completion,
        };

        MempoolService::process_input(&mut state, &command)
            .await
            .expect("Should process command");

        let result: bool = rx.await.expect("Should receive result");
        assert!(result);
    }

    #[tokio::test]
    async fn test_service_stats() {
        let tip = create_test_block_commitment(100);
        let context = create_test_context_arc_with_state(tip).await;
        let mut state = MempoolServiceState::new_with_context(context.clone(), tip);

        // Add a transaction via handle_submit_transaction
        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let tx_bytes = ssz::Encode::as_ssz_bytes(&tx);

        state
            .handle_submit_transaction(tx_bytes)
            .await
            .expect("Should add tx");

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::Stats { completion };

        MempoolService::process_input(&mut state, &command)
            .await
            .expect("Should process command");

        let stats: OLMempoolStats = rx.await.expect("Should receive stats");
        assert_eq!(stats.mempool_size(), 1);
        assert_eq!(stats.enqueues_accepted(), 1);
    }

    #[tokio::test]
    async fn test_service_set_current_slot() {
        let tip = create_test_block_commitment(100);
        let context = create_test_context_arc_with_state(tip).await;
        let mut state = MempoolServiceState::new_with_context(context, tip);

        state.set_current_tip(create_test_block_commitment(150));
        // Slot update doesn't return anything, just verify it doesn't panic
    }
}
