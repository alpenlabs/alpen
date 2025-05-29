use alloy_rpc_types::engine::ExecutionPayloadV1;
use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use reth_primitives::{Block, TransactionSigned};
use revm_primitives::{FixedBytes, B256};
use strata_primitives::{
    buf::{Buf20, Buf32},
    evm_exec::create_evm_extra_payload,
};
use strata_state::exec_update::{Op, UpdateInput};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub(crate) struct ElPayload {
    /// The parent hash of the block.
    pub parent_hash: Buf32,
    /// The fee recipient of the block.
    pub fee_recipient: Buf20,
    /// The state root of the block.
    pub state_root: Buf32,
    /// The receipts root of the block.
    pub receipts_root: Buf32,
    /// The logs bloom of the block.
    pub logs_bloom: [u8; 256],
    /// The previous randao of the block.
    pub prev_randao: Buf32,
    /// The block number.
    pub block_number: u64,
    /// The gas limit of the block.
    pub gas_limit: u64,
    /// The gas used of the block.
    pub gas_used: u64,
    /// The timestamp of the block.
    pub timestamp: u64,
    /// The extra data of the block.
    pub extra_data: Vec<u8>,
    /// The base fee per gas of the block.
    pub base_fee_per_gas: Buf32,
    /// The block hash of the block.
    pub block_hash: Buf32,
    /// The transactions of the block.
    pub transactions: Vec<Vec<u8>>,
}

#[derive(Debug, Error)]
#[allow(dead_code)]
pub(crate) enum ElPayloadError {
    #[error("Failed to extract evm block from payload: {0}")]
    BlockConversionError(String),
}

#[allow(dead_code)]
pub(crate) fn make_update_input_from_payload_and_ops(
    el_payload: ElPayload,
    ops: &[Op],
) -> Result<UpdateInput, ElPayloadError> {
    let extra_payload = create_evm_extra_payload(el_payload.block_hash);
    let v1_payload = ExecutionPayloadV1::from(el_payload);
    let evm_block: Block<TransactionSigned> = v1_payload
        .try_into_block()
        .map_err(|e| ElPayloadError::BlockConversionError(e.to_string()))?;

    Ok(UpdateInput::new(
        evm_block.number,
        ops.to_vec(),
        Buf32(*evm_block.transactions_root),
        extra_payload,
    ))
}

impl From<ExecutionPayloadV1> for ElPayload {
    fn from(val: ExecutionPayloadV1) -> Self {
        ElPayload {
            parent_hash: val.parent_hash.0.into(),
            fee_recipient: val.fee_recipient.0 .0.into(),
            state_root: val.state_root.0.into(),
            receipts_root: val.receipts_root.0.into(),
            logs_bloom: val.logs_bloom.0.into(),
            prev_randao: val.prev_randao.0.into(),
            block_number: val.block_number,
            gas_limit: val.gas_limit,
            gas_used: val.gas_used,
            timestamp: val.timestamp,
            extra_data: val.extra_data.into(),
            base_fee_per_gas: B256::from(val.base_fee_per_gas).0.into(),
            block_hash: val.block_hash.0.into(),
            transactions: val
                .transactions
                .into_iter()
                .map(|bytes| bytes.0.into())
                .collect(),
        }
    }
}

impl From<ElPayload> for ExecutionPayloadV1 {
    fn from(val: ElPayload) -> Self {
        ExecutionPayloadV1 {
            parent_hash: val.parent_hash.0.into(),
            fee_recipient: val.fee_recipient.0.into(),
            state_root: val.state_root.0.into(),
            receipts_root: val.receipts_root.0.into(),
            logs_bloom: val.logs_bloom.into(),
            prev_randao: val.prev_randao.0.into(),
            block_number: val.block_number,
            gas_limit: val.gas_limit,
            gas_used: val.gas_used,
            timestamp: val.timestamp,
            extra_data: val.extra_data.into(),
            base_fee_per_gas: FixedBytes(val.base_fee_per_gas.0).into(),
            block_hash: val.block_hash.0.into(),
            transactions: val
                .transactions
                .into_iter()
                .map(|bytes| bytes.into())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use arbitrary::{Arbitrary, Unstructured};
    use rand::{rngs::OsRng, RngCore};

    use super::*;

    #[test]
    fn into() {
        let mut rand_data = vec![0u8; 1024];
        OsRng.fill_bytes(&mut rand_data);
        let mut unstructured = Unstructured::new(&rand_data);

        let el_payload = ElPayload::arbitrary(&mut unstructured).unwrap();

        let v1_payload: ExecutionPayloadV1 = el_payload.clone().into();

        let el_payload_2: ElPayload = v1_payload.into();

        assert_eq!(el_payload, el_payload_2);
    }
}
