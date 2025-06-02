use std::sync::Arc;

use alloy_rpc_types::{
    engine::{
        ExecutionPayloadInputV2, ForkchoiceState, PayloadAttributes, PayloadId, PayloadStatusEnum,
    },
    Withdrawal,
};
use alpen_reth_evm::constants::COINBASE_ADDRESS;
use alpen_reth_node::{
    ExecutionPayloadFieldV2, StrataExecutionPayloadEnvelopeV2, StrataPayloadAttributes,
};
use futures::future::TryFutureExt;
use revm_primitives::{Address, B256};
use strata_eectl::{
    engine::{BlockStatus, ExecEngineCtl, PayloadStatus},
    errors::{EngineError, EngineResult},
    messages::{ExecPayloadData, PayloadEnv},
};
use strata_primitives::l1::BitcoinAmount;
use strata_state::{
    block::L2BlockBundle,
    bridge_ops,
    exec_update::{ELDepositData, ExecUpdate, Op, UpdateOutput},
    id::L2BlockId,
};
use strata_storage::L2BlockManager;
use tokio::{runtime::Handle, sync::Mutex};
use tracing::*;

use crate::{
    block::EVML2Block,
    el_payload::{make_update_input_from_payload_and_ops, ElPayload},
    http_client::EngineRpc,
};

#[allow(dead_code)]
fn address_from_slice(slice: &[u8]) -> Option<Address> {
    let slice: Option<[u8; 20]> = slice.try_into().ok();
    slice.map(Address::from)
}

#[allow(dead_code)]
fn sats_to_gwei(sats: u64) -> Option<u64> {
    // 1 BTC = 10^8 sats = 10^9 gwei
    sats.checked_mul(10)
}

#[allow(dead_code)]
fn gwei_to_sats(gwei: u64) -> u64 {
    // 1 BTC = 10^8 sats = 10^9 gwei
    gwei / 10
}

#[derive(Debug)]
struct StateCache {
    head_block_hash: B256,
    safe_block_hash: B256,
    finalized_block_hash: B256,
}

#[derive(Debug)]
struct RpcExecEngineInner<T: EngineRpc> {
    pub client: T,
    pub state_cache: Mutex<StateCache>,
}

impl<T: EngineRpc> RpcExecEngineInner<T> {
    #[allow(dead_code)]
    fn new(client: T, head_block_hash: B256) -> Self {
        Self {
            client,
            state_cache: Mutex::new(StateCache {
                head_block_hash,
                safe_block_hash: head_block_hash,
                finalized_block_hash: B256::ZERO,
            }),
        }
    }

    #[allow(dead_code)]
    async fn update_block_state(
        &self,
        fcs_partial: ForkchoiceStatePartial,
    ) -> EngineResult<BlockStatus> {
        let fork_choice_state = {
            // FIXME: Possibly this cache lock must be hold throughout the function call?
            let cache = self.state_cache.lock().await;
            ForkchoiceState {
                head_block_hash: fcs_partial.head_block_hash.unwrap_or(cache.head_block_hash),
                safe_block_hash: fcs_partial.safe_block_hash.unwrap_or(cache.safe_block_hash),
                finalized_block_hash: fcs_partial
                    .finalized_block_hash
                    .unwrap_or(cache.finalized_block_hash),
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
                let mut cache = self.state_cache.lock().await;
                cache.head_block_hash = fork_choice_state.head_block_hash;
                cache.safe_block_hash = fork_choice_state.safe_block_hash;
                cache.finalized_block_hash = fork_choice_state.finalized_block_hash;
                EngineResult::Ok(BlockStatus::Valid)
            }
            PayloadStatusEnum::Syncing => EngineResult::Ok(BlockStatus::Syncing),
            PayloadStatusEnum::Invalid { .. } => EngineResult::Ok(BlockStatus::Invalid),
            PayloadStatusEnum::Accepted => EngineResult::Err(EngineError::Unimplemented), /* should not be possible */
        }
    }

    #[allow(dead_code)]
    async fn build_block_from_mempool(
        &self,
        payload_env: PayloadEnv,
        prev_block: EVML2Block,
    ) -> EngineResult<u64> {
        // TODO: pass other fields from payload_env
        let withdrawals = payload_env
            .el_ops()
            .iter()
            .map(|op| match op {
                Op::Deposit(deposit_data) => Ok(Withdrawal {
                    index: deposit_data.intent_idx(),
                    address: address_from_slice(deposit_data.dest_addr()).ok_or_else(|| {
                        EngineError::InvalidAddress(deposit_data.dest_addr().to_vec())
                    })?,
                    amount: sats_to_gwei(deposit_data.amt())
                        .ok_or(EngineError::AmountConversion(deposit_data.amt()))?,
                    ..Default::default()
                }),
            })
            .collect::<Result<_, _>>()?;

        let payload_attributes = StrataPayloadAttributes::new(
            PayloadAttributes {
                // evm expects timestamp in seconds
                timestamp: payload_env.timestamp() / 1000,
                prev_randao: B256::ZERO,
                withdrawals: Some(withdrawals),
                parent_beacon_block_root: None,
                suggested_fee_recipient: COINBASE_ADDRESS,
            },
            payload_env.batch_gas_limit(),
        );

        let fcs = ForkchoiceState {
            head_block_hash: prev_block.block_hash(),
            safe_block_hash: B256::ZERO,
            finalized_block_hash: B256::ZERO,
        };

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

    #[allow(dead_code)]
    async fn get_payload_status(&self, payload_id: u64) -> EngineResult<PayloadStatus> {
        let pl_id = PayloadId::new(payload_id.to_be_bytes());
        let payload = self
            .client
            .get_payload_v2(pl_id)
            .map_err(|_| EngineError::UnknownPayloadId(payload_id))
            .await?;

        let StrataExecutionPayloadEnvelopeV2 {
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
                            withdrawal.index,
                            gwei_to_sats(withdrawal.amount),
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
        let gas_used = el_payload.gas_used;
        let update_input = make_update_input_from_payload_and_ops(el_payload, &ops)
            .map_err(|err| EngineError::Other(err.to_string()))?;

        let withdrawal_intents = rpc_withdrawal_intents
            .into_iter()
            .map(to_bridge_withdrawal_intent)
            .collect();

        let update_output =
            UpdateOutput::new_from_state(el_state_root).with_withdrawals(withdrawal_intents);

        let execution_payload_data = ExecPayloadData::new(
            ExecUpdate::new(update_input, update_output),
            accessory_data,
            ops,
        );

        Ok(PayloadStatus::Ready(execution_payload_data, gas_used))
    }

    #[allow(dead_code)]
    async fn submit_new_payload(&self, payload: ExecPayloadData) -> EngineResult<BlockStatus> {
        let Ok(el_payload) = borsh::from_slice::<ElPayload>(payload.accessory_data()) else {
            // In particular, this happens if we try to call it with for genesis block.
            warn!("submit_new_payload called with malformed block accessory, this might be a bug");
            return Ok(BlockStatus::Invalid);
        };

        // actually bridge-in deposits
        let withdrawals: Vec<Withdrawal> = payload
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Deposit(deposit_data) => Some(Withdrawal {
                    index: deposit_data.intent_idx(),
                    address: address_from_slice(deposit_data.dest_addr())?,
                    amount: sats_to_gwei(deposit_data.amt())?,
                    validator_index: 0,
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

    #[allow(dead_code)]
    async fn check_block_exists(&self, block_hash: B256) -> EngineResult<bool> {
        let block = self
            .client
            .block_by_hash(block_hash)
            .await
            .map_err(|err| EngineError::Other(err.to_string()))?;
        Ok(block.is_some())
    }
}

#[allow(dead_code)]
#[expect(missing_debug_implementations)]
pub struct RpcExecEngineCtl<T: EngineRpc> {
    inner: RpcExecEngineInner<T>,
    tokio_handle: Handle,
    l2_block_manager: Arc<L2BlockManager>,
}

impl<T: EngineRpc> RpcExecEngineCtl<T> {
    pub fn new(
        client: T,
        evm_head_block_hash: B256,
        handle: Handle,
        l2_block_manager: Arc<L2BlockManager>,
    ) -> Self {
        Self {
            inner: RpcExecEngineInner::new(client, evm_head_block_hash),
            tokio_handle: handle,
            l2_block_manager,
        }
    }
}

impl<T: EngineRpc> RpcExecEngineCtl<T> {
    #[allow(dead_code)]
    fn get_l2block(&self, l2_block_id: &L2BlockId) -> EngineResult<L2BlockBundle> {
        self.l2_block_manager
            .get_block_data_blocking(l2_block_id)
            .map_err(|err| EngineError::Other(err.to_string()))?
            .ok_or(EngineError::DbMissingBlock(*l2_block_id))
    }

    #[allow(dead_code)]
    fn get_evm_block_hash(&self, l2_block_id: &L2BlockId) -> EngineResult<B256> {
        self.get_l2block(l2_block_id)
            .and_then(|l2block| self.get_block_info(l2block))
            .map(|evm_block| evm_block.block_hash())
    }

    #[allow(dead_code)]
    fn get_block_info(&self, l2block: L2BlockBundle) -> EngineResult<EVML2Block> {
        EVML2Block::try_extract(&l2block).map_err(|err| EngineError::Other(err.to_string()))
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
        let prev_block = EVML2Block::try_extract(&prev_l2block)
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

        // Send forkchoiceupdate { headBlockHash, safeBlockHash: 0, finalizedBlockHash: 0 }

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

    fn check_block_exists(&self, id: L2BlockId) -> EngineResult<bool> {
        let block = self
            .get_l2block(&id)
            .and_then(|l2block| self.get_block_info(l2block))?;
        let block_hash = block.block_hash();
        self.tokio_handle
            .block_on(self.inner.check_block_exists(block_hash))
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

fn to_bridge_withdrawal_intent(
    rpc_withdrawal_intent: alpen_reth_node::WithdrawalIntent,
) -> bridge_ops::WithdrawalIntent {
    let alpen_reth_node::WithdrawalIntent {
        amt,
        destination,
        withdrawal_txid,
    } = rpc_withdrawal_intent;
    bridge_ops::WithdrawalIntent::new(BitcoinAmount::from_sat(amt), destination, withdrawal_txid)
}

#[cfg(test)]
mod tests {
    use alloy_rpc_types::engine::{ExecutionPayloadV1, ForkchoiceUpdated};
    use alpen_reth_node::{ExecutionPayloadEnvelopeV2, ExecutionPayloadFieldV2};
    use rand::{rngs::OsRng, Rng};
    use revm_primitives::{alloy_primitives::Bloom, Bytes, FixedBytes, U256};
    use strata_eectl::{errors::EngineResult, messages::PayloadEnv};
    use strata_primitives::buf::Buf32;
    use strata_state::block::{L2Block, L2BlockAccessory};

    use super::*;
    use crate::http_client::MockEngineRpc;

    fn random_el_payload() -> ElPayload {
        random_execution_payload_v1().into()
    }

    fn random_execution_payload_v1() -> ExecutionPayloadV1 {
        ExecutionPayloadV1 {
            parent_hash: B256::random(),
            fee_recipient: Address::random(),
            state_root: B256::random(),
            receipts_root: B256::random(),
            logs_bloom: Bloom::random(),
            prev_randao: B256::random(),
            block_number: OsRng.gen(),
            gas_limit: 200_000u64,
            gas_used: 10_000u64,
            timestamp: OsRng.gen(),
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

        let initial_head_block_hash = B256::random();

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, initial_head_block_hash);

        let fcs_update = ForkchoiceStatePartial {
            head_block_hash: Some(B256::random()),
            safe_block_hash: None,
            finalized_block_hash: None,
        };

        let result = rpc_exec_engine_inner.update_block_state(fcs_update).await;

        assert!(matches!(result, EngineResult::Ok(BlockStatus::Valid)));
        assert!(
            *rpc_exec_engine_inner
                .state_cache
                .lock()
                .await
                .head_block_hash
                == fcs_update.head_block_hash.unwrap(),
            "cached head block hash updated"
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

        let initial_head_block_hash = B256::random();

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, initial_head_block_hash);

        let fcs_update = ForkchoiceStatePartial {
            head_block_hash: Some(B256::random()),
            safe_block_hash: None,
            finalized_block_hash: None,
        };

        let result = rpc_exec_engine_inner.update_block_state(fcs_update).await;

        assert!(matches!(result, EngineResult::Ok(BlockStatus::Invalid)));
        assert!(
            *rpc_exec_engine_inner
                .state_cache
                .lock()
                .await
                .head_block_hash
                == initial_head_block_hash,
            "cached head block hash remains unchanged"
        )
    }

    #[tokio::test]
    async fn test_build_block_from_mempool() {
        let mut mock_client = MockEngineRpc::new();
        let head_block_hash = B256::random();

        mock_client
            .expect_fork_choice_updated_v2()
            .returning(move |_, _| {
                Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Valid)
                    .with_payload_id(PayloadId::new([1u8; 8])))
            });

        let el_payload = random_el_payload();

        let mut arb = strata_test_utils::ArbitraryGenerator::new();
        let l2block: L2Block = arb.generate();
        let accessory = L2BlockAccessory::new(borsh::to_vec(&el_payload).unwrap(), 0);
        let l2block_bundle = L2BlockBundle::new(l2block, accessory);

        let evm_l2_block = EVML2Block::try_extract(&l2block_bundle).unwrap();

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, head_block_hash);

        let timestamp = 0;
        let el_ops = vec![];
        let safe_l1_block = Buf32(FixedBytes::<32>::random().into());
        let prev_l2_block = Buf32(FixedBytes::<32>::random().into()).into();

        let payload_env = PayloadEnv::new(timestamp, prev_l2_block, safe_l1_block, el_ops, None);

        let result = rpc_exec_engine_inner
            .build_block_from_mempool(payload_env, evm_l2_block)
            .await;

        assert!(result.is_ok());
        assert!(
            *rpc_exec_engine_inner
                .state_cache
                .lock()
                .await
                .head_block_hash
                == head_block_hash,
            "cached head block remains unchanged"
        );
    }

    #[tokio::test]
    async fn test_get_payload_status() {
        let mut mock_client = MockEngineRpc::new();
        let head_block_hash = B256::random();

        mock_client.expect_get_payload_v2().returning(move |_| {
            Ok(StrataExecutionPayloadEnvelopeV2 {
                inner: ExecutionPayloadEnvelopeV2 {
                    execution_payload: ExecutionPayloadFieldV2::V1(random_execution_payload_v1()),
                    block_value: U256::from(100),
                },
                withdrawal_intents: vec![],
            })
        });

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, head_block_hash);

        let result = rpc_exec_engine_inner.get_payload_status(0).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_submit_new_payload() {
        let mut mock_client = MockEngineRpc::new();
        let head_block_hash = B256::random();

        let el_payload = ElPayload {
            base_fee_per_gas: Buf32(FixedBytes::<32>::from(U256::from(10)).into()),
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

        let update_input = make_update_input_from_payload_and_ops(el_payload, &[]).unwrap();
        let update_output = UpdateOutput::new_from_state(Buf32::zero());

        let payload_data = ExecPayloadData::new(
            ExecUpdate::new(update_input, update_output),
            accessory_data,
            vec![],
        );

        mock_client.expect_new_payload_v2().returning(move |_| {
            Ok(alloy_rpc_types::engine::PayloadStatus {
                status: PayloadStatusEnum::Valid,
                latest_valid_hash: None,
            })
        });

        let rpc_exec_engine_inner = RpcExecEngineInner::new(mock_client, head_block_hash);

        let result = rpc_exec_engine_inner.submit_new_payload(payload_data).await;

        assert!(matches!(result, EngineResult::Ok(BlockStatus::Valid)));
    }
}
