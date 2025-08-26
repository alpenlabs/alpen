use alloy_eips::BlockNumHash;
use alloy_primitives::{Address, B256};
use alloy_rpc_types::{
    engine::{ForkchoiceState, PayloadAttributes},
    Withdrawal,
};
use alpen_reth_node::{
    payload::{AlpenBuiltPayload, AlpenPayloadBuilderAttributes},
    AlpenEngineTypes, AlpenPayloadAttributes,
};
use reth_chain_state::CanonicalInMemoryState;
use reth_node_builder::{
    BeaconConsensusEngineHandle, BuiltPayload, EngineApiMessageVersion, PayloadBuilderAttributes,
    PayloadTypes,
};
use reth_payload_builder::PayloadBuilderHandle;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub(crate) enum EnginePayloadError {
    #[error("todo")]
    Other,
}

pub(crate) trait ExecutionEngine<TEnginePayload> {
    async fn submit_payload(&self, payload: TEnginePayload) -> eyre::Result<()>;

    async fn update_consenesus_state(&self, state: ForkchoiceState) -> eyre::Result<()>;
}

pub(crate) trait PayloadBuilderEngine<TEnginePayload>:
    ExecutionEngine<TEnginePayload>
{
    async fn build_payload(
        &self,
        build_attrs: EnginePayloadAttributes,
    ) -> eyre::Result<TEnginePayload>;
}

#[derive(Debug, Clone)]
pub(crate) struct DepositInfo {
    index: u64,
    address: Address,
    amount: u64,
}

impl DepositInfo {
    fn index(&self) -> u64 {
        self.index
    }

    fn address(&self) -> Address {
        self.address
    }

    fn amount(&self) -> u64 {
        self.amount
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EnginePayloadAttributes {
    pub(crate) parent: B256,
    pub(crate) timestamp: u64,
    pub(crate) deposits: Vec<DepositInfo>,
}

impl EnginePayloadAttributes {
    fn parent(&self) -> B256 {
        self.parent
    }

    fn timestamp(&self) -> u64 {
        self.timestamp
    }

    fn deposits(&self) -> &[DepositInfo] {
        &self.deposits
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AlpenRethExecEngine {
    payload_builder_handle: PayloadBuilderHandle<AlpenEngineTypes>,
    beacon_engine_handle: BeaconConsensusEngineHandle<AlpenEngineTypes>,
}

impl AlpenRethExecEngine {
    pub fn new(
        payload_builder_handle: PayloadBuilderHandle<AlpenEngineTypes>,
        beacon_engine_handle: BeaconConsensusEngineHandle<AlpenEngineTypes>,
    ) -> Self {
        Self {
            payload_builder_handle,
            beacon_engine_handle,
        }
    }
}

impl ExecutionEngine<AlpenBuiltPayload> for AlpenRethExecEngine {
    async fn submit_payload(&self, payload: AlpenBuiltPayload) -> eyre::Result<()> {
        self.beacon_engine_handle
            .new_payload(AlpenEngineTypes::block_to_payload(
                payload.block().to_owned(),
            ))
            .await?;

        // TODO: handle invalid blocks

        Ok(())
    }

    async fn update_consenesus_state(&self, state: ForkchoiceState) -> eyre::Result<()> {
        self.beacon_engine_handle
            .fork_choice_updated(state, None, EngineApiMessageVersion::V4)
            .await?;

        // TODO: handle invalid blocks

        Ok(())
    }
}

impl PayloadBuilderEngine<AlpenBuiltPayload> for AlpenRethExecEngine {
    async fn build_payload(
        &self,
        build_attrs: EnginePayloadAttributes,
    ) -> eyre::Result<AlpenBuiltPayload> {
        let payload_attrs = AlpenPayloadAttributes::new_from_eth(PayloadAttributes {
            timestamp: build_attrs.timestamp(),
            // IMPORTANT: post cancun will payload build will fail without
            // parent_beacon_block_root
            parent_beacon_block_root: Some(B256::ZERO),
            prev_randao: B256::ZERO,
            // TODO: get from config
            suggested_fee_recipient: Address::ZERO,
            withdrawals: Some(
                build_attrs
                    .deposits()
                    .iter()
                    .map(|deposit| Withdrawal {
                        index: deposit.index(),
                        validator_index: 0,
                        address: deposit.address(),
                        amount: deposit.amount(),
                    })
                    .collect(),
            ),
        });

        let payload_builder_attrs =
            AlpenPayloadBuilderAttributes::try_new(build_attrs.parent(), payload_attrs, 0)?;

        let payload_id = self
            .payload_builder_handle
            .send_new_payload(payload_builder_attrs)
            .await
            .expect("should send payload correctly")?;

        let payload = self
            .payload_builder_handle
            .resolve_kind(payload_id, reth_node_builder::PayloadKind::WaitForPending)
            .await
            .expect("should resolve payload")
            .expect("should build payload");

        Ok(payload)
    }
}

pub(crate) trait ChainStateProvider {
    fn head_block_hash(&self) -> BlockNumHash;
    fn safe_block_hash(&self) -> Option<BlockNumHash>;
    fn finalized_block_hash(&self) -> Option<BlockNumHash>;
}

#[derive(Debug, Clone)]
pub(crate) struct RethChainStateProvider {
    pub(crate) canonical_in_memory_state: CanonicalInMemoryState,
    // pub(crate) provider:
}

impl ChainStateProvider for RethChainStateProvider {
    fn head_block_hash(&self) -> BlockNumHash {
        self.canonical_in_memory_state.chain_info().into()
    }
    fn safe_block_hash(&self) -> Option<BlockNumHash> {
        self.canonical_in_memory_state.get_safe_num_hash()
    }
    fn finalized_block_hash(&self) -> Option<BlockNumHash> {
        self.canonical_in_memory_state.get_finalized_num_hash()
    }
}
