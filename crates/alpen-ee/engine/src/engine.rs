use alloy_rpc_types_engine::ForkchoiceState;
use alpen_ee_common::{ExecutionEngine, ExecutionEngineError};
use alpen_reth_node::{AlpenBuiltPayload, AlpenEngineTypes};
use async_trait::async_trait;
use reth_node_builder::{
    BuiltPayload, ConsensusEngineHandle, EngineApiMessageVersion, PayloadTypes,
};
use strata_common::retry::{
    policies::ExponentialBackoff, retry_with_backoff_async, Backoff,
    DEFAULT_ENGINE_CALL_MAX_RETRIES,
};
use tracing::{debug, error, warn};

/// Execution engine implementation using Reth for Alpen EE.
#[derive(Debug, Clone)]
pub struct AlpenRethExecEngine {
    beacon_engine_handle: ConsensusEngineHandle<AlpenEngineTypes>,
}

impl AlpenRethExecEngine {
    /// Creates a new Alpen Reth execution engine.
    pub fn new(beacon_engine_handle: ConsensusEngineHandle<AlpenEngineTypes>) -> Self {
        Self {
            beacon_engine_handle,
        }
    }
}

#[async_trait]
impl ExecutionEngine for AlpenRethExecEngine {
    type TEnginePayload = AlpenBuiltPayload;

    async fn submit_payload(&self, payload: AlpenBuiltPayload) -> Result<(), ExecutionEngineError> {
        let backoff = ExponentialBackoff::default();
        let mut delay = backoff.base_delay_ms();

        for attempt in 0..=DEFAULT_ENGINE_CALL_MAX_RETRIES {
            let result = self
                .beacon_engine_handle
                .new_payload(AlpenEngineTypes::block_to_payload(
                    payload.block().to_owned(),
                ))
                .await
                .map_err(|e| ExecutionEngineError::communication(e.to_string()))
                .and_then(|status| match status.status {
                    alloy_rpc_types_engine::PayloadStatusEnum::Valid => Ok(()),
                    alloy_rpc_types_engine::PayloadStatusEnum::Invalid { validation_error } => {
                        Err(ExecutionEngineError::invalid_payload(validation_error))
                    }
                    alloy_rpc_types_engine::PayloadStatusEnum::Syncing => {
                        Err(ExecutionEngineError::engine_syncing(
                            alloy_rpc_types_engine::PayloadStatusEnum::Syncing.as_str(),
                        ))
                    }
                    alloy_rpc_types_engine::PayloadStatusEnum::Accepted => Ok(()),
                });

            match result {
                Ok(value) => return Ok(value),
                Err(err) if attempt < DEFAULT_ENGINE_CALL_MAX_RETRIES && err.is_retryable() => {
                    warn!(
                        "Attempt {} failed with {err:?} while running exec_engine_submit_payload. Retrying in {delay:?}ms",
                        attempt + 1,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay = backoff.next_delay_ms(delay);
                }
                Err(err) => {
                    if attempt >= DEFAULT_ENGINE_CALL_MAX_RETRIES {
                        error!("Max retries exceeded while running exec_engine_submit_payload, returning with the last error");
                    }
                    return Err(err);
                }
            }
        }

        unreachable!()
    }

    async fn update_consensus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError> {
        retry_with_backoff_async(
            "exec_engine_update_consensus_state",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                debug!(?state, "Sending fork choice state to beacon");
                self.beacon_engine_handle
                    .fork_choice_updated(state, None, EngineApiMessageVersion::V4)
                    .await
                    .map(|_| ())
                    .map_err(|e| ExecutionEngineError::fork_choice_update(e.to_string()))
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use alloy_consensus::Header;
    use alloy_rpc_types_engine::{PayloadStatus, PayloadStatusEnum};
    use alpen_ee_common::ExecutionEngineError;
    use alpen_reth_node::WithdrawalIntent;
    use reth_ethereum_engine_primitives::EthBuiltPayload;
    use reth_node_builder::BeaconEngineMessage;
    use reth_primitives::{BlockBody, SealedBlock};
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    fn dummy_payload() -> AlpenBuiltPayload {
        let header = Header {
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp: 1,
            base_fee_per_gas: Some(0),
            withdrawals_root: Some(Default::default()),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: Some(Default::default()),
            requests_hash: Some(Default::default()),
            ..Default::default()
        };
        let block = alloy_consensus::Block {
            header,
            body: BlockBody {
                transactions: Vec::new(),
                ommers: Vec::new(),
                withdrawals: Some(vec![].into()),
            },
        };
        let sealed_block: Arc<SealedBlock<_>> = Arc::new(SealedBlock::seal_slow(block));
        let eth_payload =
            EthBuiltPayload::new(Default::default(), sealed_block, Default::default(), None);
        AlpenBuiltPayload::new(eth_payload, Vec::<WithdrawalIntent>::new())
    }

    #[tokio::test]
    async fn submit_payload_maps_invalid_payload_status_to_invalid_payload_error() {
        let (tx, mut rx) = unbounded_channel();
        let engine = AlpenRethExecEngine::new(ConsensusEngineHandle::new(tx));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let responder = tokio::spawn(async move {
            let Some(BeaconEngineMessage::NewPayload { tx, .. }) = rx.recv().await else {
                panic!("expected new payload message");
            };
            attempts_clone.fetch_add(1, Ordering::SeqCst);
            tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Invalid {
                validation_error: "invalid block".into(),
            })))
            .expect("receiver should still be alive");
        });

        let result = engine.submit_payload(dummy_payload()).await;

        responder.await.expect("responder task should complete");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        match result {
            Err(ExecutionEngineError::InvalidPayload(msg)) => {
                assert_eq!(msg, "invalid block");
            }
            other => panic!("unexpected submit payload result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_payload_maps_syncing_status_to_engine_syncing_error() {
        let (tx, mut rx) = unbounded_channel();
        let engine = AlpenRethExecEngine::new(ConsensusEngineHandle::new(tx));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();
        let max_retries = usize::from(DEFAULT_ENGINE_CALL_MAX_RETRIES);

        let responder = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                let BeaconEngineMessage::NewPayload { tx, .. } = message else {
                    panic!("expected new payload message");
                };
                attempts_clone.fetch_add(1, Ordering::SeqCst);
                tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Syncing)))
                    .expect("receiver should still be alive");

                if attempts_clone.load(Ordering::SeqCst) > max_retries {
                    break;
                }
            }
        });

        let result = engine.submit_payload(dummy_payload()).await;

        responder.await.expect("responder task should complete");
        assert_eq!(attempts.load(Ordering::SeqCst), max_retries + 1);

        match result {
            Err(ExecutionEngineError::EngineSyncing(msg)) => {
                assert_eq!(msg, "SYNCING");
            }
            other => panic!("unexpected submit payload result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_payload_retries_syncing_status_until_valid() {
        let (tx, mut rx) = unbounded_channel();
        let engine = AlpenRethExecEngine::new(ConsensusEngineHandle::new(tx));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let responder = tokio::spawn(async move {
            let mut should_sync = true;
            while let Some(message) = rx.recv().await {
                let BeaconEngineMessage::NewPayload { tx, .. } = message else {
                    panic!("expected new payload message");
                };
                attempts_clone.fetch_add(1, Ordering::SeqCst);

                if should_sync {
                    should_sync = false;
                    tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Syncing)))
                        .expect("receiver should still be alive");
                } else {
                    tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)))
                        .expect("receiver should still be alive");
                    break;
                }
            }
        });

        tokio::time::timeout(
            Duration::from_secs(5),
            engine.submit_payload(dummy_payload()),
        )
        .await
        .expect("submit payload should finish within the timeout")
        .expect("syncing status should retry and then succeed");

        responder.await.expect("responder task should complete");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn submit_payload_treats_accepted_status_as_success() {
        let (tx, mut rx) = unbounded_channel();
        let engine = AlpenRethExecEngine::new(ConsensusEngineHandle::new(tx));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let responder = tokio::spawn(async move {
            let Some(BeaconEngineMessage::NewPayload { tx, .. }) = rx.recv().await else {
                panic!("expected new payload message");
            };
            attempts_clone.fetch_add(1, Ordering::SeqCst);
            tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Accepted)))
                .expect("receiver should still be alive");
        });

        tokio::time::timeout(
            Duration::from_secs(5),
            engine.submit_payload(dummy_payload()),
        )
        .await
        .expect("submit payload should finish within the timeout")
        .expect("accepted status should be treated as a non-fatal success");

        responder.await.expect("responder task should complete");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn submit_payload_retries_communication_error_until_valid() {
        let (tx, mut rx) = unbounded_channel();
        let engine = AlpenRethExecEngine::new(ConsensusEngineHandle::new(tx));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let responder = tokio::spawn(async move {
            let mut should_drop_response = true;
            while let Some(message) = rx.recv().await {
                let BeaconEngineMessage::NewPayload { tx, .. } = message else {
                    panic!("expected new payload message");
                };
                attempts_clone.fetch_add(1, Ordering::SeqCst);

                if should_drop_response {
                    should_drop_response = false;
                    drop(tx);
                } else {
                    tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)))
                        .expect("receiver should still be alive");
                    break;
                }
            }
        });

        tokio::time::timeout(
            Duration::from_secs(5),
            engine.submit_payload(dummy_payload()),
        )
        .await
        .expect("submit payload should finish within the timeout")
        .expect("communication error should retry and then succeed");

        responder.await.expect("responder task should complete");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}
