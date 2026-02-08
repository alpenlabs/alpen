use std::{error, fmt::Debug};

use alloy_eips::eip7685::Requests;
use alloy_primitives::{B256, U256};
use alloy_rlp::Decodable;
use alloy_rpc_types_engine::PayloadId;
use alpen_reth_node::{AlpenBuiltPayload, WithdrawalIntent};
use bincode::{deserialize, serialize};
use reth_ethereum_engine_primitives::{BlobSidecars, EthBuiltPayload};
use reth_ethereum_primitives::{Block, EthPrimitives};
use reth_node_builder::{BuiltPayload, NodePrimitives};
use reth_primitives_traits::{Block as BlockTrait, SealedBlock};
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
    #[error(transparent)]
    Serialization(#[from] bincode::Error),
    #[error("RLP decoding failed: {0}")]
    RlpDecode(#[from] alloy_rlp::Error),
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
        Ok(serialize(&serializable)?)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error> {
        let serializable = deserialize::<SerializablePayload>(bytes)?;
        serializable.try_into()
    }
}

/// Internal representation of a payload for bincode serialization.
///
/// The block is stored as RLP-encoded bytes because `SealedBlock`'s serde
/// implementation uses iterators that bincode cannot serialize when the
/// block contains transactions.
#[derive(Debug, Serialize, Deserialize)]
struct SerializablePayload {
    payload_id: PayloadId,
    /// RLP-encoded block bytes.
    block_rlp: Vec<u8>,
    /// Block hash (used to re-seal the block on deserialization).
    block_hash: B256,
    fees: U256,
    requests: Option<Requests>,
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

        // RLP encode the block for bincode serialization
        let sealed_block = eth_built_payload.block();
        let block_hash = sealed_block.hash();
        let block_rlp = alloy_rlp::encode(sealed_block.clone_block());

        Ok(SerializablePayload {
            payload_id: eth_built_payload.id(),
            block_rlp,
            block_hash,
            fees: eth_built_payload.fees(),
            requests: eth_built_payload.requests().clone(),
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
            block_hash,
            fees,
            requests,
            withdrawal_intents,
        } = value;

        // Decode the RLP-encoded block and seal it with the stored hash
        let block = Block::decode(&mut block_rlp.as_slice())?;
        let sealed_block: SealedBlock<<EthPrimitives as NodePrimitives>::Block> =
            block.seal_unchecked(block_hash);

        let eth_built_payload =
            EthBuiltPayload::new(payload_id, sealed_block.into(), fees, requests);

        Ok(AlpenBuiltPayload::new(
            eth_built_payload,
            withdrawal_intents,
        ))
    }
}
