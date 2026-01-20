//! Mempool service implementation.

use std::marker::PhantomData;

use serde::Serialize;
use strata_ol_state_types::StateProvider;
use strata_service::{AsyncService, Response, Service};

use crate::{
    MempoolCommand, builder::MempoolInputMessage, state::MempoolServiceState, types::OLMempoolStats,
};

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
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            MempoolInputMessage::Command(cmd) => match cmd {
                MempoolCommand::SubmitTransaction { tx, completion } => {
                    let result = state.handle_submit_transaction(tx.clone()).await;
                    completion.send(result).await;
                }

                MempoolCommand::GetTransactions { completion, limit } => {
                    let result = state.handle_get_transactions(*limit).await;
                    completion.send(result).await;
                }

                MempoolCommand::RemoveTransactions { txs, completion } => {
                    let result = state.handle_remove_transactions(txs.clone());
                    completion.send(result).await;
                }
            },

            MempoolInputMessage::ChainUpdate(update) => {
                state.handle_chain_update(update.tip()).await?;
            }
        }

        Ok(Response::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_identifiers::OLTxId;
    use strata_service::CommandCompletionSender;
    use tokio::{runtime::Runtime, sync::oneshot};

    use super::*;
    use crate::{
        OLMempoolResult, OLMempoolTransaction,
        test_utils::{
            create_test_block_commitment, create_test_context, create_test_snark_tx_with_seq_no,
            create_test_state_provider,
        },
        types::OLMempoolConfig,
    };

    #[tokio::test]
    async fn test_service_submit_transaction() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));

        let mut state = MempoolServiceState::new_with_context(context, tip)
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

        MempoolService::process_input(&mut state, &MempoolInputMessage::Command(command))
            .await
            .expect("Should process command");

        let result: OLMempoolResult<OLTxId> = rx.await.expect("Should receive result");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected_txid);
    }

    proptest::proptest! {
        #[test]
        fn test_service_get_transactions_with_limit(limit in 0usize..10) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let tip = create_test_block_commitment(100);
                let provider = Arc::new(create_test_state_provider(tip));
                let context = Arc::new(create_test_context(OLMempoolConfig::default(), provider.clone()));

                let mut state = MempoolServiceState::new_with_context(context.clone(), tip).await.unwrap();

                // Add some transactions via handle_submit_transaction
                // Use sequential seq_nos (0, 1) for the same account to pass gap checking
                let tx1 = create_test_snark_tx_with_seq_no(1, 0);
                let tx2 = create_test_snark_tx_with_seq_no(1, 1);
                state
                    .handle_submit_transaction(Box::new(tx1))
                    .await
                    .expect("Should add tx1");
                state
                    .handle_submit_transaction(Box::new(tx2))
                    .await
                    .expect("Should add tx2");

                let (tx_sender, rx) = oneshot::channel();
                let completion = CommandCompletionSender::new(tx_sender);

                let command = MempoolCommand::GetTransactions {
                    completion,
                    limit,
                };

                MempoolService::process_input(&mut state, &MempoolInputMessage::Command(command))
                    .await
                    .expect("Should process command");

                let result: OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>> =
                    rx.await.expect("Should receive result");
                assert!(result.is_ok());
                let txs = result.unwrap();
                #[expect(clippy::absolute_paths, reason = "qualified min avoids ambiguity")]
                let expected_len = std::cmp::min(limit, 2);
                assert_eq!(txs.len(), expected_len);
            });
        }
    }

    #[tokio::test]
    async fn test_service_remove_transactions() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));

        let mut state = MempoolServiceState::new_with_context(context.clone(), tip)
            .await
            .unwrap();

        // Add a transaction via handle_submit_transaction
        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let txid = tx.compute_txid();
        state
            .handle_submit_transaction(Box::new(tx))
            .await
            .expect("Should add tx");

        let (tx_sender, rx) = oneshot::channel();
        let completion = CommandCompletionSender::new(tx_sender);

        let command = MempoolCommand::RemoveTransactions {
            txs: vec![(txid, crate::MempoolTxRemovalReason::Included)],
            completion,
        };

        MempoolService::process_input(&mut state, &MempoolInputMessage::Command(command))
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
    async fn test_service_stats() {
        let tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));
        let mut state = MempoolServiceState::new_with_context(context.clone(), tip)
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
}
