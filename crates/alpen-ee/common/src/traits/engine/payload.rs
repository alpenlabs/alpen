use std::{error, fmt::Debug};

use alloy_eips::eip7685::Requests;
use alloy_primitives::{B256, U256};
use alloy_rlp::{Decodable, Encodable};
use alloy_rpc_types_engine::PayloadId;
use alpen_reth_node::{AlpenBuiltPayload, WithdrawalIntent};
use reth_ethereum_engine_primitives::{BlobSidecars, EthBuiltPayload};
use reth_ethereum_primitives::EthPrimitives;
use reth_node_builder::{BuiltPayload, NodePrimitives};
use reth_primitives_traits::SealedBlock;
use serde::{Deserialize, Serialize};
use strata_acct_types::Hash;
use thiserror::Error;
use tracing::error;

/// Trait for engine payloads that can be serialized and provide block metadata.
pub trait EnginePayload: Sized + Clone {
    type Error: error::Error + Send + Sync + 'static;

    /// Returns the block number of this payload.
    fn blocknum(&self) -> u64;
    /// Returns the block hash of this payload.
    fn blockhash(&self) -> Hash;
    /// Returns the withdrawal intents included in this payload.
    fn withdrawal_intents(&self) -> &[WithdrawalIntent];

    /// Serializes this payload to bytes.
    fn to_bytes(&self) -> Result<Vec<u8>, Self::Error>;
    /// Deserializes a payload from bytes.
    fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error>;
}

/// Errors that can occur when working with Alpen engine payloads.
#[derive(Debug, Error)]
pub enum AlpenEnginePayloadError {
    #[error("expected blob sidecars to be empty; blockhash: {0}")]
    BlobSidecarsNotEmpty(B256),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("RLP encoding error: {0}")]
    RlpEncode(#[from] alloy_rlp::Error),
}

impl EnginePayload for AlpenBuiltPayload {
    type Error = AlpenEnginePayloadError;

    fn blocknum(&self) -> u64 {
        self.block().number
    }

    fn blockhash(&self) -> Hash {
        self.block().hash().0.into()
    }

    fn withdrawal_intents(&self) -> &[WithdrawalIntent] {
        self.withdrawal_intents()
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Self::Error> {
        let serializable = SerializablePayload::try_from(self.clone())?;
        serde_json::to_vec(&serializable).map_err(|e| {
            error!(
                blockhash = %self.block().hash(),
                block_number = self.block().number,
                tx_count = self.block().body().transactions.len(),
                error = %e,
                "failed to serialize payload"
            );
            AlpenEnginePayloadError::Serialization(e)
        })
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error> {
        let serializable = serde_json::from_slice::<SerializablePayload>(bytes)?;
        serializable.try_into()
    }
}

type EthBlock = <EthPrimitives as NodePrimitives>::Block;

/// Internal representation of a payload for serialization.
///
/// Uses RLP encoding for the block (which Ethereum types support natively)
/// and serde_json for the wrapper structure.
#[derive(Debug, Serialize, Deserialize)]
struct SerializablePayload {
    payload_id: PayloadId,
    /// RLP-encoded sealed block
    block_rlp: Vec<u8>,
    fees: U256,
    /// Requests stored as Vec of raw bytes
    requests: Option<Vec<Vec<u8>>>,
    withdrawal_intents: Vec<WithdrawalIntent>,
}

impl TryFrom<AlpenBuiltPayload> for SerializablePayload {
    type Error = AlpenEnginePayloadError;

    fn try_from(value: AlpenBuiltPayload) -> Result<Self, Self::Error> {
        let (eth_built_payload, withdrawal_intents) = value.into_parts();

        if !matches!(eth_built_payload.sidecars(), BlobSidecars::Empty) {
            let blockhash = eth_built_payload.block().hash();
            error!(%blockhash, "expected payload sidecars to be empty");
            return Err(AlpenEnginePayloadError::BlobSidecarsNotEmpty(blockhash));
        }

        // Encode block using RLP
        let block = eth_built_payload.block();
        let mut block_rlp = Vec::new();
        block.encode(&mut block_rlp);

        // Store requests as Vec of raw bytes
        let requests = eth_built_payload
            .requests()
            .as_ref()
            .map(|r| r.iter().map(|b| b.to_vec()).collect::<Vec<_>>());

        Ok(SerializablePayload {
            payload_id: eth_built_payload.id(),
            block_rlp,
            fees: eth_built_payload.fees(),
            requests,
            withdrawal_intents,
        })
    }
}

impl TryFrom<SerializablePayload> for AlpenBuiltPayload {
    type Error = AlpenEnginePayloadError;

    fn try_from(value: SerializablePayload) -> Result<Self, Self::Error> {
        let SerializablePayload {
            payload_id,
            block_rlp,
            fees,
            requests,
            withdrawal_intents,
        } = value;

        // Decode block from RLP
        let block = SealedBlock::<EthBlock>::decode(&mut block_rlp.as_slice())?;

        // Convert requests back to Requests type
        let requests = requests.map(|reqs| {
            Requests::new(
                reqs.into_iter()
                    .map(alloy_primitives::Bytes::from)
                    .collect(),
            )
        });

        let eth_built_payload = EthBuiltPayload::new(payload_id, block.into(), fees, requests);

        Ok(AlpenBuiltPayload::new(
            eth_built_payload,
            withdrawal_intents,
        ))
    }
}
