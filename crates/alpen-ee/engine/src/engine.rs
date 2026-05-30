use alloy_rpc_types_engine::ForkchoiceState;
use alpen_ee_common::{ExecutionEngine, ExecutionEngineError};
use alpen_reth_node::{AlpenBuiltPayload, AlpenEngineTypes};
use async_trait::async_trait;
use reth_node_builder::{
    BeaconForkChoiceUpdateError, BuiltPayload, ConsensusEngineHandle, EngineApiMessageVersion,
    PayloadTypes,
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

fn map_forkchoice_error(err: BeaconForkChoiceUpdateError) -> ExecutionEngineError {
    match err {
        BeaconForkChoiceUpdateError::EngineUnavailable => {
            ExecutionEngineError::communication(err.to_string())
        }
        BeaconForkChoiceUpdateError::Internal(_) => {
            ExecutionEngineError::communication(err.to_string())
        }
        BeaconForkChoiceUpdateError::ForkchoiceUpdateError(_) => {
            ExecutionEngineError::fork_choice_update(err.to_string())
        }
    }
}

#[async_trait]
impl ExecutionEngine for AlpenRethExecEngine {
    type TEnginePayload = AlpenBuiltPayload;

    async fn submit_payload(&self, payload: AlpenBuiltPayload) -> Result<(), ExecutionEngineError> {
        retry_with_backoff_async(
            "exec_engine_submit_payload",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                self.beacon_engine_handle
                    .new_payload(AlpenEngineTypes::block_to_payload(
                        payload.block().to_owned(),
                    ))
                    .await
                    .map(|_| ())
                    .map_err(|e| ExecutionEngineError::payload_submission(e.to_string()))
            },
        )
        .await
    }

    async fn update_consensus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError> {
        let backoff = ExponentialBackoff::default();
        let mut delay = backoff.base_delay_ms();

        for attempt in 0..=DEFAULT_ENGINE_CALL_MAX_RETRIES {
            let result = {
                debug!(?state, "Sending fork choice state to beacon");
                self.beacon_engine_handle
                    .fork_choice_updated(state, None, EngineApiMessageVersion::V4)
                    .await
                    .map(|_| ())
                    .map_err(map_forkchoice_error)
            };

            match result {
                Ok(value) => return Ok(value),
                Err(err) if attempt < DEFAULT_ENGINE_CALL_MAX_RETRIES && err.is_retryable() => {
                    warn!(
                        "Attempt {} failed with {err:?} while running exec_engine_update_consensus_state. Retrying in {delay:?}ms",
                        attempt + 1,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay = backoff.next_delay_ms(delay);
                }
                Err(err) => {
                    if attempt >= DEFAULT_ENGINE_CALL_MAX_RETRIES {
                        error!("Max retries exceeded while running exec_engine_update_consensus_state, returning with the last error");
                    }
                    return Err(err);
                }
            }
        }

        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use alloy_primitives::B256;
    use alloy_rpc_types_engine::{ForkchoiceState, PayloadStatus, PayloadStatusEnum};
    use alpen_ee_common::ExecutionEngineError;
    use reth_node_builder::{BeaconEngineMessage, OnForkChoiceUpdated};
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    #[tokio::test]
    async fn update_consensus_state_maps_invalid_state_to_forkchoice_error_without_retry() {
        let (tx, mut rx) = unbounded_channel();
        let engine = AlpenRethExecEngine::new(ConsensusEngineHandle::new(tx));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let responder = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                let BeaconEngineMessage::ForkchoiceUpdated { tx, .. } = message else {
                    panic!("expected forkchoice update message");
                };
                attempts_clone.fetch_add(1, Ordering::SeqCst);
                tx.send(Ok(OnForkChoiceUpdated::invalid_state()))
                    .expect("receiver should still be alive");
            }
        });

        let err = engine
            .update_consensus_state(ForkchoiceState {
                head_block_hash: B256::from([9u8; 32]),
                safe_block_hash: B256::from([5u8; 32]),
                finalized_block_hash: B256::from([6u8; 32]),
            })
            .await
            .expect_err("invalid forkchoice state should surface as an error");

        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        drop(engine);
        responder.await.expect("responder task should complete");

        match err {
            ExecutionEngineError::ForkChoiceUpdate(msg) => {
                assert!(
                    !msg.is_empty(),
                    "fork choice update error message should not be empty"
                );
                assert_eq!(msg, "forkchoice update error: invalid forkchoice state");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_consensus_state_retries_engine_unavailable_errors() {
        let (tx, mut rx) = unbounded_channel();
        let engine = AlpenRethExecEngine::new(ConsensusEngineHandle::new(tx));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let responder = tokio::spawn(async move {
            let mut should_fail = true;
            while let Some(message) = rx.recv().await {
                let BeaconEngineMessage::ForkchoiceUpdated { tx, .. } = message else {
                    panic!("expected forkchoice update message");
                };
                attempts_clone.fetch_add(1, Ordering::SeqCst);

                if should_fail {
                    should_fail = false;
                    drop(tx);
                } else {
                    tx.send(Ok(OnForkChoiceUpdated::valid(PayloadStatus::from_status(
                        PayloadStatusEnum::Valid,
                    ))))
                    .expect("receiver should still be alive");
                    break;
                }
            }
        });

        let update_task = tokio::spawn({
            let engine = engine.clone();
            async move {
                engine
                    .update_consensus_state(ForkchoiceState {
                        head_block_hash: B256::from([9u8; 32]),
                        safe_block_hash: B256::from([5u8; 32]),
                        finalized_block_hash: B256::from([6u8; 32]),
                    })
                    .await
            }
        });

        tokio::time::timeout(Duration::from_secs(5), update_task)
            .await
            .expect("update task should finish within the timeout")
            .expect("update task should complete")
            .expect("retryable engine-unavailable errors should succeed after a retry");
        responder.await.expect("responder task should complete");
        assert!(
            attempts.load(Ordering::SeqCst) == 2,
            "retryable engine-unavailable errors should perform exactly one retry in this test"
        );
    }
}
