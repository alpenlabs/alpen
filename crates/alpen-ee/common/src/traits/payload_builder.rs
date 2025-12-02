use alpen_reth_node::{AlpenBuiltPayload, WithdrawalIntent};
use async_trait::async_trait;
use reth_node_builder::BuiltPayload;
use strata_acct_types::Hash;

use crate::types::payload_builder::PayloadBuildAttributes;

#[async_trait]
pub trait PayloadBuilderEngine {
    type TEnginePayload: EnginePayload;

    async fn build_payload(
        &self,
        build_attrs: PayloadBuildAttributes,
    ) -> eyre::Result<Self::TEnginePayload>;
}

pub trait EnginePayload: Sized {
    fn blocknum(&self) -> u64;
    fn blockhash(&self) -> Hash;
    fn withdrawal_intents(&self) -> &[WithdrawalIntent];

    fn to_bytes(&self) -> Vec<u8>;
    fn from_bytes(bytes: &[u8]) -> Option<Self>;
}

impl EnginePayload for AlpenBuiltPayload {
    fn blocknum(&self) -> u64 {
        self.block().number
    }

    fn blockhash(&self) -> Hash {
        self.block().hash().0
    }

    fn withdrawal_intents(&self) -> &[WithdrawalIntent] {
        self.withdrawal_intents()
    }

    fn to_bytes(&self) -> Vec<u8> {
        todo!()
    }

    fn from_bytes(_bytes: &[u8]) -> Option<Self> {
        todo!()
    }
}
