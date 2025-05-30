use std::sync::Arc;

use alloy_rpc_types::engine::{
    ExecutionPayload, ExecutionPayloadEnvelopeV3, ExecutionPayloadEnvelopeV4,
    ExecutionPayloadSidecar, ExecutionPayloadV1, PayloadError,
};
use reth::rpc::compat::engine::payload::block_to_payload;
use reth_chainspec::ChainSpec;
use reth_node_api::{
    payload::PayloadTypes, validate_version_specific_fields, AddOnsContext, BuiltPayload,
    EngineApiMessageVersion, EngineObjectValidationError, EngineTypes, EngineValidator,
    NodePrimitives, PayloadOrAttributes, PayloadValidator,
};
use reth_node_builder::{rpc::EngineValidatorBuilder, FullNodeComponents, NodeTypesWithEngine};
use reth_payload_validator::ExecutionPayloadValidator;
use reth_primitives::{Block, SealedBlockFor};
use serde::{Deserialize, Serialize};

use super::payload::{StrataBuiltPayload, StrataPayloadBuilderAttributes};
use crate::{node::StrataPrimitives, StrataExecutionPayloadEnvelopeV2, StrataPayloadAttributes};

/// Custom engine types for strata to use custom payload attributes and payload
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct StrataEngineTypes<T: PayloadTypes = StrataPayloadTypes> {
    _marker: std::marker::PhantomData<T>,
}

impl<T: PayloadTypes> PayloadTypes for StrataEngineTypes<T> {
    type BuiltPayload = T::BuiltPayload;
    type PayloadAttributes = T::PayloadAttributes;
    type PayloadBuilderAttributes = T::PayloadBuilderAttributes;
}

impl<T: PayloadTypes> EngineTypes for StrataEngineTypes<T>
where
    T::BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = reth_primitives::Block>>
        + TryInto<ExecutionPayloadV1>
        + TryInto<StrataExecutionPayloadEnvelopeV2>
        + TryInto<ExecutionPayloadEnvelopeV3>
        + TryInto<ExecutionPayloadEnvelopeV4>,
{
    type ExecutionPayloadEnvelopeV1 = ExecutionPayloadV1;
    type ExecutionPayloadEnvelopeV2 = StrataExecutionPayloadEnvelopeV2;
    type ExecutionPayloadEnvelopeV3 = ExecutionPayloadEnvelopeV3;
    type ExecutionPayloadEnvelopeV4 = ExecutionPayloadEnvelopeV4;

    fn block_to_payload(
        block: SealedBlockFor<
            <<Self::BuiltPayload as BuiltPayload>::Primitives as NodePrimitives>::Block,
        >,
    ) -> (ExecutionPayload, ExecutionPayloadSidecar) {
        block_to_payload(block)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StrataPayloadTypes;

impl PayloadTypes for StrataPayloadTypes {
    type BuiltPayload = StrataBuiltPayload;
    type PayloadAttributes = StrataPayloadAttributes;
    type PayloadBuilderAttributes = StrataPayloadBuilderAttributes;
}

/// Strata engine validator
#[derive(Debug, Clone)]
pub struct StrataEngineValidator {
    inner: ExecutionPayloadValidator<ChainSpec>,
}

impl StrataEngineValidator {
    /// Instantiates a new validator.
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            inner: ExecutionPayloadValidator::new(chain_spec),
        }
    }

    /// Returns the chain spec used by the validator.
    #[inline]
    #[expect(clippy::missing_const_for_fn)]
    fn chain_spec(&self) -> &ChainSpec {
        self.inner.chain_spec()
    }
}

impl PayloadValidator for StrataEngineValidator {
    type Block = Block;

    fn ensure_well_formed_payload(
        &self,
        payload: ExecutionPayload,
        sidecar: ExecutionPayloadSidecar,
    ) -> Result<SealedBlockFor<Self::Block>, PayloadError> {
        self.inner.ensure_well_formed_payload(payload, sidecar)
    }
}

impl<T> EngineValidator<T> for StrataEngineValidator
where
    T: EngineTypes<PayloadAttributes = StrataPayloadAttributes>,
{
    fn validate_version_specific_fields(
        &self,
        version: EngineApiMessageVersion,
        payload_or_attrs: PayloadOrAttributes<'_, T::PayloadAttributes>,
    ) -> Result<(), EngineObjectValidationError> {
        validate_version_specific_fields(self.chain_spec(), version, payload_or_attrs)
    }

    fn ensure_well_formed_attributes(
        &self,
        version: EngineApiMessageVersion,
        attributes: &T::PayloadAttributes,
    ) -> Result<(), EngineObjectValidationError> {
        validate_version_specific_fields(self.chain_spec(), version, attributes.into())?;

        Ok(())
    }
}

/// Builder for [`StrataEngineValidator`].
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct StrataEngineValidatorBuilder;

impl<Node, Types> EngineValidatorBuilder<Node> for StrataEngineValidatorBuilder
where
    Types: NodeTypesWithEngine<
        ChainSpec = ChainSpec,
        Primitives = StrataPrimitives,
        Engine = StrataEngineTypes,
    >,
    Node: FullNodeComponents<Types = Types>,
{
    type Validator = StrataEngineValidator;

    async fn build(self, ctx: &AddOnsContext<'_, Node>) -> eyre::Result<Self::Validator> {
        Ok(StrataEngineValidator::new(ctx.config.chain.clone()))
    }
}
