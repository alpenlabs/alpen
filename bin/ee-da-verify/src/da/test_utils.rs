use core::mem::size_of;

use alpen_ee_common::{parse_chunk_header, BatchId, DaBlob, EvmHeaderSummary};
use bitcoin::{
    hashes::{sha256, Hash as _},
    Wtxid,
};
use proptest::prelude::*;

use crate::l1::scan::RevealRecord;

const BASE_TIMESTAMP: u64 = 1_700_000_000;
const BASE_GAS_LIMIT: u64 = 30_000_000;
const MIN_MULTI_CHUNK_BYTECODE_LEN: usize = 400_000;
const MAX_MULTI_CHUNK_BYTECODE_LEN: usize = 450_000;

/// Builds a batch id fixture derived from `block_num`.
pub(crate) fn test_batch_id(block_num: u64) -> BatchId {
    let prev_block = hash_from_block_num(block_num, 0x11);
    let last_block = hash_from_block_num(block_num, 0x22);
    BatchId::from_parts(prev_block.into(), last_block.into())
}

/// Builds an EE header fixture for DA-stage tests.
pub(crate) fn test_evm_header(block_num: u64) -> EvmHeaderSummary {
    let block_delta = block_num % 1_000_000;
    let gas_used = BASE_GAS_LIMIT / 2 + (block_delta % 1_000);

    EvmHeaderSummary {
        block_num,
        timestamp: BASE_TIMESTAMP + block_delta,
        base_fee: 1_000_000_000 + block_delta,
        gas_used,
        gas_limit: BASE_GAS_LIMIT + (block_delta % 1_000),
    }
}

/// Builds a minimal DA blob fixture with an empty state diff.
pub(crate) fn make_test_blob(block_num: u64) -> DaBlob {
    DaBlob {
        batch_id: test_batch_id(block_num),
        evm_header: test_evm_header(block_num),
        state_diff: Default::default(),
    }
}

/// Builds a DA blob fixture large enough to span multiple chunks.
pub(crate) fn make_multi_chunk_test_blob(
    block_num: u64,
    bytecode_len: usize,
    fill_byte: u8,
) -> DaBlob {
    let mut blob = make_test_blob(block_num);
    blob.state_diff
        .deployed_bytecodes
        .insert(Default::default(), vec![fill_byte; bytecode_len].into());
    blob
}

/// Strategy for bytecode sizes that force multi-chunk DA encoding.
pub(crate) fn multi_chunk_bytecode_len_strategy() -> impl Strategy<Value = usize> {
    MIN_MULTI_CHUNK_BYTECODE_LEN..=MAX_MULTI_CHUNK_BYTECODE_LEN
}

/// Builds reveal records from encoded chunk bytes for DA-stage tests.
pub(crate) fn build_reveal_records_from_chunk_bytes(chunks: Vec<Vec<u8>>) -> Vec<RevealRecord> {
    let mut previous_wtxid = [0u8; 32];
    let mut records = Vec::with_capacity(chunks.len());

    for (index, chunk_bytes) in chunks.into_iter().enumerate() {
        let chunk_header = parse_chunk_header(&chunk_bytes).expect("fixture chunk header is valid");
        let wtxid_bytes = synthetic_wtxid_bytes(index, &chunk_bytes);

        records.push(RevealRecord {
            wtxid: Wtxid::from_byte_array(wtxid_bytes),
            prev_wtxid: previous_wtxid,
            chunk_header,
            chunk_bytes,
            block_tx_index: index,
        });
        previous_wtxid = wtxid_bytes;
    }

    records
}

fn synthetic_wtxid_bytes(index: usize, chunk_bytes: &[u8]) -> [u8; 32] {
    let mut seed = Vec::with_capacity(size_of::<usize>() + chunk_bytes.len());
    seed.extend_from_slice(&index.to_le_bytes());
    seed.extend_from_slice(chunk_bytes);
    sha256::Hash::hash(&seed).to_byte_array()
}

fn hash_from_block_num(block_num: u64, tag: u8) -> [u8; 32] {
    let mut hash = [tag; 32];
    hash[24..].copy_from_slice(&block_num.to_be_bytes());
    hash
}
