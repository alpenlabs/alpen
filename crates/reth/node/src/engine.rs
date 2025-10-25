use std::sync::Arc;

use alloy_rpc_types::engine::{
    payload::ExecutionData, ExecutionPayload, ExecutionPayloadEnvelopeV3, ExecutionPayloadV1,
};
use reth_chainspec::ChainSpec;
use reth_ethereum_payload_builder::EthereumExecutionPayloadValidator;
use reth_node_api::{
    payload::PayloadTypes, validate_version_specific_fields, AddOnsContext, BuiltPayload,
    EngineApiMessageVersion, EngineObjectValidationError, EngineTypes, EngineValidator,
    FullNodeComponents, InvalidPayloadAttributesError, NewPayloadError, NodeTypes,
    PayloadOrAttributes, PayloadValidator,
};
use reth_node_builder::rpc::EngineValidatorBuilder;
use reth_primitives::{Block, EthPrimitives, NodePrimitives, RecoveredBlock, SealedBlock};
use serde::{Deserialize, Serialize};

use crate::{
    payload::{AlpenBuiltPayload, AlpenExecutionPayloadEnvelopeV4, AlpenPayloadBuilderAttributes},
    AlpenExecutionPayloadEnvelopeV2, AlpenPayloadAttributes,
};

/// Custom engine types for strata to use custom payload attributes and payload
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AlpenEngineTypes {}

impl PayloadTypes for AlpenEngineTypes {
    type BuiltPayload = AlpenBuiltPayload;
    type ExecutionData = ExecutionData;
    type PayloadAttributes = AlpenPayloadAttributes;
    type PayloadBuilderAttributes = AlpenPayloadBuilderAttributes;

    fn block_to_payload(
        block: SealedBlock<
            <<Self::BuiltPayload as BuiltPayload>::Primitives as NodePrimitives>::Block,
        >,
    ) -> Self::ExecutionData {
        let (payload, sidecar) =
            ExecutionPayload::from_block_unchecked(block.hash(), &block.into_block());
        ExecutionData { payload, sidecar }
    }
}

impl EngineTypes for AlpenEngineTypes {
    type ExecutionPayloadEnvelopeV1 = ExecutionPayloadV1;
    type ExecutionPayloadEnvelopeV2 = AlpenExecutionPayloadEnvelopeV2;
    type ExecutionPayloadEnvelopeV3 = ExecutionPayloadEnvelopeV3;
    type ExecutionPayloadEnvelopeV4 = AlpenExecutionPayloadEnvelopeV4;
}

/// Strata engine validator
#[derive(Debug, Clone)]
pub struct AlpenEngineValidator {
    inner: EthereumExecutionPayloadValidator<ChainSpec>,
}

impl AlpenEngineValidator {
    /// Instantiates a new validator.
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            inner: EthereumExecutionPayloadValidator::new(chain_spec),
        }
    }

    /// Returns the chain spec used by the validator.
    #[inline]
    fn chain_spec(&self) -> &ChainSpec {
        self.inner.chain_spec()
    }
}

impl PayloadValidator for AlpenEngineValidator {
    type Block = Block;
    type ExecutionData = ExecutionData;

    fn ensure_well_formed_payload(
        &self,
        payload: ExecutionData,
    ) -> Result<RecoveredBlock<Self::Block>, NewPayloadError> {
        let sealed_block = self.inner.ensure_well_formed_payload(payload)?;
        sealed_block
            .try_recover()
            .map_err(|e| NewPayloadError::Other(e.into()))
    }
}

impl<T> EngineValidator<T> for AlpenEngineValidator
where
    T: EngineTypes<PayloadAttributes = AlpenPayloadAttributes, ExecutionData = ExecutionData>,
{
    fn validate_version_specific_fields(
        &self,
        version: EngineApiMessageVersion,
        payload_or_attrs: PayloadOrAttributes<'_, Self::ExecutionData, T::PayloadAttributes>,
    ) -> Result<(), EngineObjectValidationError> {
        validate_version_specific_fields(self.chain_spec(), version, payload_or_attrs)
    }

    fn ensure_well_formed_attributes(
        &self,
        version: EngineApiMessageVersion,
        attributes: &T::PayloadAttributes,
    ) -> Result<(), EngineObjectValidationError> {
        validate_version_specific_fields(
            self.chain_spec(),
            version,
            PayloadOrAttributes::<Self::ExecutionData, T::PayloadAttributes>::PayloadAttributes(
                attributes,
            ),
        )?;

        Ok(())
    }

    fn validate_payload_attributes_against_header(
        &self,
        _attr: &<T as PayloadTypes>::PayloadAttributes,
        _header: &<Self::Block as reth::api::Block>::Header,
    ) -> Result<(), InvalidPayloadAttributesError> {
        // skip default timestamp validation
        Ok(())
    }
}

/// Custom engine validator builder
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct AlpenEngineValidatorBuilder;

impl<N> EngineValidatorBuilder<N> for AlpenEngineValidatorBuilder
where
    N: FullNodeComponents<
        Types: NodeTypes<
            Payload = AlpenEngineTypes,
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
        >,
    >,
{
    type Validator = AlpenEngineValidator;

    async fn build(self, ctx: &AddOnsContext<'_, N>) -> eyre::Result<Self::Validator> {
        Ok(AlpenEngineValidator::new(ctx.config.chain.clone()))
    }
}
