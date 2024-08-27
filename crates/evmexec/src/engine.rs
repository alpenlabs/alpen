use std::sync::Arc;

use alpen_express_eectl::{
    engine::{BlockStatus, ExecEngineCtl, PayloadStatus},
    errors::{EngineError, EngineResult},
    messages::{ELDepositData, ExecPayloadData, Op, PayloadEnv},
};
use alpen_express_primitives::buf::Buf64;
use alpen_express_state::{
    block::L2BlockBundle,
    bridge_ops,
    exec_update::{ExecUpdate, UpdateInput, UpdateOutput},
    id::L2BlockId,
};
use express_reth_node::{ExpressExecutionPayloadEnvelopeV2, ExpressPayloadAttributes};
use express_storage::L2BlockManager;
use futures::future::TryFutureExt;
use reth_primitives::{Address, B256};
use reth_rpc_types::{
    engine::{
        ExecutionPayloadFieldV2, ExecutionPayloadInputV2, ForkchoiceState, PayloadAttributes,
        PayloadId, PayloadStatusEnum,
    },
    Withdrawal,
};
use tokio::{runtime::Handle, sync::Mutex};

use crate::{block::EVML2Block, el_payload::ElPayload, http_client::EngineRpc};

fn address_from_slice(slice: &[u8]) -> Option<Address> {
    let slice: Option<[u8; 20]> = slice.try_into().ok();
    slice.map(Address::from)
}

struct RpcExecEngineInner<T: EngineRpc> {
    pub client: T,
    pub fork_choice_state: Mutex<ForkchoiceState>,
}

impl<T: EngineRpc> RpcExecEngineInner<T> {
    fn new(client: T, fork_choice_state: ForkchoiceState) -> Self {
        Self {
            client,
            fork_choice_state: Mutex::new(fork_choice_state),
        }
    }

    async fn update_block_state(
        &self,
        fcs_partial: ForkchoiceStatePartial,
    ) -> EngineResult<BlockStatus> {
        let fork_choice_state = {
            let existing = self.fork_choice_state.lock().await;
            ForkchoiceState {
                head_block_hash: fcs_partial
                    .head_block_hash
                    .unwrap_or(existing.head_block_hash),
                safe_block_hash: fcs_partial
                    .safe_block_hash
                    .unwrap_or(existing.safe_block_hash),
                finalized_block_hash: fcs_partial
                    .finalized_block_hash
                    .unwrap_or(existing.finalized_block_hash),
            }
        };

        let fork_choice_result = self
            .client
            .fork_choice_updated_v2(fork_choice_state, None)
            .await;

        let update_status =
            fork_choice_result.map_err(|err| EngineError::Other(err.to_string()))?;

        match update_status.payload_status.status {
            PayloadStatusEnum::Valid => {
                *self.fork_choice_state.lock().await = fork_choice_state;
                EngineResult::Ok(BlockStatus::Valid)
            }
            PayloadStatusEnum::Syncing => EngineResult::Ok(BlockStatus::Syncing),
            PayloadStatusEnum::Invalid { .. } => EngineResult::Ok(BlockStatus::Invalid),
            PayloadStatusEnum::Accepted => EngineResult::Err(EngineError::Unimplemented), /* should not be possible */
        }
    }

    async fn build_block_from_mempool(
        &self,
        payload_env: PayloadEnv,
        prev_block: EVML2Block,
    ) -> EngineResult<u64> {
        // TODO: pass other fields from payload_env
        let withdrawals: Vec<Withdrawal> = payload_env
            .el_ops()
            .iter()
            .filter_map(|op| match op {
                Op::Deposit(deposit_data) => Some(Withdrawal {
                    address: address_from_slice(deposit_data.dest_addr())?,
                    amount: deposit_data.amt(),
                    ..Default::default()
                }),
            })
            .collect();

        let payload_attributes = ExpressPayloadAttributes::new_from_eth(PayloadAttributes {
            // evm expects timestamp in seconds
            timestamp: payload_env.timestamp() / 1000,
            prev_randao: B256::ZERO,
            withdrawals: Some(withdrawals),
            parent_beacon_block_root: None,
            suggested_fee_recipient: Address::ZERO,
        });

        let mut fcs = *self.fork_choice_state.lock().await;
        fcs.head_block_hash = prev_block.block_hash();

        let forkchoice_result = self
            .client
            .fork_choice_updated_v2(fcs, Some(payload_attributes))
            .await;

        // TODO: correct error type
        let update_status = forkchoice_result.map_err(|err| EngineError::Other(err.to_string()))?;

        let payload_id: PayloadId = update_status
            .payload_id
            .ok_or(EngineError::Other("payload_id missing".into()))?; // should never happen

        let raw_id: [u8; 8] = payload_id.0.into();

        Ok(u64::from_be_bytes(raw_id))
    }

    async fn get_payload_status(&self, payload_id: u64) -> EngineResult<PayloadStatus> {
        let pl_id = PayloadId::new(payload_id.to_be_bytes());
        let payload = self
            .client
            .get_payload_v2(pl_id)
            .map_err(|_| EngineError::UnknownPayloadId(payload_id))
            .await?;

        let ExpressExecutionPayloadEnvelopeV2 {
            inner: execution_payload_v2,
            withdrawal_intents: rpc_withdrawal_intents,
        } = payload;

        let (el_payload, ops) = match execution_payload_v2.execution_payload {
            ExecutionPayloadFieldV2::V1(payload) => {
                let el_payload: ElPayload = payload.into();

                (el_payload, vec![])
            }
            ExecutionPayloadFieldV2::V2(payload) => {
                let ops = payload
                    .withdrawals
                    .iter()
                    .map(|withdrawal| {
                        Op::Deposit(ELDepositData::new(
                            withdrawal.amount,
                            withdrawal.address.as_slice().to_vec(),
                        ))
                    })
                    .collect();

                let el_payload: ElPayload = payload.payload_inner.into();

                (el_payload, ops)
            }
        };

        let el_state_root = el_payload.state_root;
        let accessory_data = borsh::to_vec(&el_payload).unwrap();
        let update_input =
            UpdateInput::try_from(el_payload).map_err(|err| EngineError::Other(err.to_string()))?;

        let withdrawal_intents = rpc_withdrawal_intents
            .into_iter()
            .map(to_bridge_withdrawal_intents)
            .collect();

        let update_output =
            UpdateOutput::new_from_state(el_state_root).with_withdrawals(withdrawal_intents);

        let execution_payload_data = ExecPayloadData::new(
            ExecUpdate::new(update_input, update_output),
            accessory_data,
            ops,
        );

        Ok(PayloadStatus::Ready(execution_payload_data))
    }

    async fn submit_new_payload(&self, payload: ExecPayloadData) -> EngineResult<BlockStatus> {
        let el_payload = borsh::from_slice::<ElPayload>(payload.accessory_data())
            .map_err(|_| EngineError::Other("Invalid payload".to_string()))?;

        // actually bridge-in deposits
        let withdrawals: Vec<Withdrawal> = payload
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Deposit(deposit_data) => Some(Withdrawal {
                    address: address_from_slice(deposit_data.dest_addr())?,
                    amount: deposit_data.amt(),
                    ..Default::default()
                }),
            })
            .collect();

        let v2_payload = ExecutionPayloadInputV2 {
            execution_payload: el_payload.into(),
            withdrawals: Some(withdrawals),
        };

        let payload_status_result = self.client.new_payload_v2(v2_payload).await;

        let payload_status =
            payload_status_result.map_err(|err| EngineError::Other(err.to_string()))?;

        match payload_status.status {
            PayloadStatusEnum::Valid => EngineResult::Ok(BlockStatus::Valid),
            PayloadStatusEnum::Syncing => EngineResult::Ok(BlockStatus::Syncing),
            PayloadStatusEnum::Invalid { .. } => EngineResult::Ok(BlockStatus::Invalid),
            PayloadStatusEnum::Accepted => EngineResult::Err(EngineError::Unimplemented), // TODO
        }
    }
}

pub struct RpcExecEngineCtl<T: EngineRpc> {
    inner: RpcExecEngineInner<T>,
    tokio_handle: Handle,
    l2_block_manager: Arc<L2BlockManager>,
}

impl<T: EngineRpc> RpcExecEngineCtl<T> {
    pub fn new(
        client: T,
        fork_choice_state: ForkchoiceState,
        handle: Handle,
        l2_block_manager: Arc<L2BlockManager>,
    ) -> Self {
        Self {
            inner: RpcExecEngineInner::new(client, fork_choice_state),
            tokio_handle: handle,
            l2_block_manager,
        }
    }
}

impl<T: EngineRpc> RpcExecEngineCtl<T> {
    fn get_l2block(&self, l2_block_id: &L2BlockId) -> anyhow::Result<L2BlockBundle> {
        self.l2_block_manager
            .get_block_blocking(l2_block_id)?
            .ok_or(anyhow::anyhow!("missing L2Block"))
    }

    fn get_evm_block_hash(&self, l2_block_id: &L2BlockId) -> anyhow::Result<B256> {
        self.get_l2block(l2_block_id)
            .and_then(|l2block| self.get_block_info(l2block))
            .map(|evm_block| evm_block.block_hash())
    }

    fn get_block_info(&self, l2block: L2BlockBundle) -> anyhow::Result<EVML2Block> {
        EVML2Block::try_from(l2block).map_err(anyhow::Error::msg)
    }
}

impl<T: EngineRpc> ExecEngineCtl for RpcExecEngineCtl<T> {
    fn submit_payload(&self, payload: ExecPayloadData) -> EngineResult<BlockStatus> {
        self.tokio_handle
            .block_on(self.inner.submit_new_payload(payload))
    }

    fn prepare_payload(&self, env: PayloadEnv) -> EngineResult<u64> {
        let prev_l2block = self
            .get_l2block(env.prev_l2_block_id())
            .map_err(|err| EngineError::Other(err.to_string()))?;
        let prev_block = EVML2Block::try_from(prev_l2block)
            .map_err(|err| EngineError::Other(err.to_string()))?;
        self.tokio_handle
            .block_on(self.inner.build_block_from_mempool(env, prev_block))
    }

    fn get_payload_status(&self, id: u64) -> EngineResult<PayloadStatus> {
        self.tokio_handle
            .block_on(self.inner.get_payload_status(id))
    }

    fn update_head_block(&self, id: L2BlockId) -> EngineResult<()> {
        let block_hash = self
            .get_evm_block_hash(&id)
            .map_err(|err| EngineError::Other(err.to_string()))?;

        self.tokio_handle.block_on(async {
            let fork_choice_state = ForkchoiceStatePartial {
                head_block_hash: Some(block_hash),
                ..Default::default()
            };
            self.inner
                .update_block_state(fork_choice_state)
                .await
                .map(|_| ())
        })
    }

    fn update_safe_block(&self, id: L2BlockId) -> EngineResult<()> {
        let block_hash = self
            .get_evm_block_hash(&id)
            .map_err(|err| EngineError::Other(err.to_string()))?;

        self.tokio_handle.block_on(async {
            let fork_choice_state = ForkchoiceStatePartial {
                // NOTE: update_head_block is not called currently; so update head and safe block
                // together
                head_block_hash: Some(block_hash),
                safe_block_hash: Some(block_hash),
                ..Default::default()
            };
            self.inner
                .update_block_state(fork_choice_state)
                .await
                .map(|_| ())
        })
    }

    fn update_finalized_block(&self, id: L2BlockId) -> EngineResult<()> {
        let block_hash = self
            .get_evm_block_hash(&id)
            .map_err(|err| EngineError::Other(err.to_string()))?;

        self.tokio_handle.block_on(async {
            let fork_choice_state = ForkchoiceStatePartial {
                finalized_block_hash: Some(block_hash),
                ..Default::default()
            };
            self.inner
                .update_block_state(fork_choice_state)
                .await
                .map(|_| ())
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct ForkchoiceStatePartial {
    /// Hash of the head block.
    pub head_block_hash: Option<B256>,
    /// Hash of the safe block.
    pub safe_block_hash: Option<B256>,
    /// Hash of finalized block.
    pub finalized_block_hash: Option<B256>,
}

fn to_bridge_withdrawal_intents(
    rpc_withdrawal_intent: express_reth_node::WithdrawalIntent,
) -> bridge_ops::WithdrawalIntent {
    let express_reth_node::WithdrawalIntent { amt, dest_pk } = rpc_withdrawal_intent;
    bridge_ops::WithdrawalIntent::new(amt, Buf64(dest_pk))
}

#[cfg(test)]
mod tests {
    use alpen_express_eectl::{errors::EngineResult, messages::PayloadEnv};
    use alpen_express_primitives::buf::Buf32;
    use alpen_express_state::block::{L2Block, L2BlockAccessory};
    use rand::Rng;
    use reth_primitives::{revm_primitives::FixedBytes, Bloom, Bytes, U256};
    use reth_rpc_types::{
        engine::{ExecutionPayloadEnvelopeV2, ForkchoiceUpdated},
        ExecutionPayloadV1,
    };

    use super::*;
    use crate::http_client::MockEngineRpc;

    fn random_el_payload() -> ElPayload {
        random_execution_payload_v1().into()
    }

    fn random_execution_payload_v1() -> ExecutionPayloadV1 {
        let mut rng = rand::thread_rng();

        ExecutionPayloadV1 {
            parent_hash: B256::random(),
            fee_recipient: Address::random(),
            state_root: B256::random(),
            receipts_root: B256::random(),
            logs_bloom: Bloom::random(),
            prev_randao: B256::random(),
            block_number: rng.gen(),
            gas_limit: 200_000u64,
            gas_used: 10_000u64,
            timestamp: rng.gen(),
            extra_data: Bytes::new(),
            base_fee_per_gas: U256::from(50),
            block_hash: B256::random(),
            transactions: vec![],
        }
    }

    #[tokio::test]
    async fn test_update_block_state_success() {
        let mut mock_client = MockEngineRpc::new();

        mock_client
            .expect_fork_choice_updated_v2()
            .returning(move |_, _| Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Valid)));

        let initial_fcs = ForkchoiceState {
            head_block_hash: B256::random(),
            safe_block_hash: B256::random(),
            finalized_block_hash: B256::random(),
        };

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, initial_fcs);

        let fcs_update = ForkchoiceStatePartial {
            head_block_hash: Some(B256::random()),
            safe_block_hash: None,
            finalized_block_hash: None,
        };

        let result = rpc_exec_engine_inner.update_block_state(fcs_update).await;

        assert!(matches!(result, EngineResult::Ok(BlockStatus::Valid)));
        assert!(
            *rpc_exec_engine_inner.fork_choice_state.lock().await
                == ForkchoiceState {
                    head_block_hash: fcs_update.head_block_hash.unwrap(),
                    safe_block_hash: initial_fcs.safe_block_hash,
                    finalized_block_hash: initial_fcs.finalized_block_hash,
                }
        )
    }

    #[tokio::test]
    async fn test_update_block_state_failed() {
        let mut mock_client = MockEngineRpc::new();

        mock_client
            .expect_fork_choice_updated_v2()
            .returning(move |_, _| {
                Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Invalid {
                    validation_error: "foo".to_string(),
                }))
            });

        let initial_fcs = ForkchoiceState {
            head_block_hash: B256::random(),
            safe_block_hash: B256::random(),
            finalized_block_hash: B256::random(),
        };

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, initial_fcs);

        let fcs_update = ForkchoiceStatePartial {
            head_block_hash: Some(B256::random()),
            safe_block_hash: None,
            finalized_block_hash: None,
        };

        let result = rpc_exec_engine_inner.update_block_state(fcs_update).await;

        assert!(matches!(result, EngineResult::Ok(BlockStatus::Invalid)));
        assert!(*rpc_exec_engine_inner.fork_choice_state.lock().await == initial_fcs)
    }

    #[tokio::test]
    async fn test_build_block_from_mempool() {
        let mut mock_client = MockEngineRpc::new();
        let fcs = ForkchoiceState::default();

        mock_client
            .expect_fork_choice_updated_v2()
            .returning(move |_, _| {
                Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Valid)
                    .with_payload_id(PayloadId::new([1u8; 8])))
            });

        let el_payload = random_el_payload();

        let arb = alpen_test_utils::ArbitraryGenerator::new();
        let l2block: L2Block = arb.generate();
        let accessory = L2BlockAccessory::new(borsh::to_vec(&el_payload).unwrap());
        let l2block_bundle = L2BlockBundle::new(l2block, accessory);

        let evm_l2_block = EVML2Block::try_from(l2block_bundle.clone()).unwrap();

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, fcs);

        let timestamp = 0;
        let el_ops = vec![];
        let safe_l1_block = Buf32(FixedBytes::<32>::random());
        let prev_l2_block = Buf32(FixedBytes::<32>::random()).into();

        let payload_env = PayloadEnv::new(timestamp, prev_l2_block, safe_l1_block, el_ops);

        let result = rpc_exec_engine_inner
            .build_block_from_mempool(payload_env, evm_l2_block)
            .await;

        assert!(result.is_ok());
        // let exec_payload = ExecutionPayloadV1::from(el_payload);
        assert!(
            *rpc_exec_engine_inner.fork_choice_state.lock().await == ForkchoiceState::default()
        );
    }

    #[tokio::test]
    async fn test_get_payload_status() {
        let mut mock_client = MockEngineRpc::new();
        let fcs = ForkchoiceState::default();

        mock_client.expect_get_payload_v2().returning(move |_| {
            Ok(ExpressExecutionPayloadEnvelopeV2 {
                inner: ExecutionPayloadEnvelopeV2 {
                    execution_payload: ExecutionPayloadFieldV2::V1(random_execution_payload_v1()),
                    block_value: U256::from(100),
                },
                withdrawal_intents: vec![],
            })
        });

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, fcs);

        let result = rpc_exec_engine_inner.get_payload_status(0).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_submit_new_payload() {
        let mut mock_client = MockEngineRpc::new();
        let fcs = ForkchoiceState::default();

        let el_payload = ElPayload {
            base_fee_per_gas: Buf32(U256::from(10).into()),
            parent_hash: Default::default(),
            fee_recipient: Default::default(),
            state_root: Default::default(),
            receipts_root: Default::default(),
            logs_bloom: [0u8; 256],
            prev_randao: Default::default(),
            block_number: Default::default(),
            gas_limit: Default::default(),
            gas_used: Default::default(),
            timestamp: Default::default(),
            extra_data: Default::default(),
            block_hash: Default::default(),
            transactions: Default::default(),
        };
        let accessory_data = borsh::to_vec(&el_payload).unwrap();
        let update_input = UpdateInput::try_from(el_payload).unwrap();
        let update_output = UpdateOutput::new_from_state(Buf32::zero());

        let payload_data = ExecPayloadData::new(
            ExecUpdate::new(update_input, update_output),
            accessory_data,
            vec![],
        );

        mock_client.expect_new_payload_v2().returning(move |_| {
            Ok(reth_rpc_types::engine::PayloadStatus {
                status: PayloadStatusEnum::Valid,
                latest_valid_hash: None,
            })
        });

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, fcs);

        let result = rpc_exec_engine_inner.submit_new_payload(payload_data).await;

        assert!(matches!(result, EngineResult::Ok(BlockStatus::Valid)));
    }
}
