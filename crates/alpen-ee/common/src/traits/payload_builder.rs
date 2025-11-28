use async_trait::async_trait;

use crate::types::payload_builder::PayloadBuildAttributes;

#[async_trait]
pub trait PayloadBuilderEngine<TEnginePayload> {
    async fn build_payload(
        &self,
        build_attrs: PayloadBuildAttributes,
    ) -> eyre::Result<TEnginePayload>;
}
