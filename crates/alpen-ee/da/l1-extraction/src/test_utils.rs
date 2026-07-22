use alpen_ee_da_types::{DaBlob, EvmHeaderSummary};
use bitcoin::{
    block::{Header, Version},
    hashes::{sha256, Hash},
    pow::CompactTarget,
    secp256k1::XOnlyPublicKey,
    Block, BlockHash, Transaction, TxMerkleNode, Txid,
};
use proptest::prelude::*;
use strata_l1_commit_reveal_fmt::test_utils as commit_reveal_fixtures;

use crate::ParsedEnvelope;

const BASE_TIMESTAMP: u64 = 1_700_000_000;
const BASE_GAS_LIMIT: u64 = 30_000_000;
const MIN_MULTI_CHUNK_BYTECODE_LEN: usize = 400_000;
const MAX_MULTI_CHUNK_BYTECODE_LEN: usize = 450_000;

pub(crate) fn build_block_with_txs(txs: Vec<Transaction>) -> Block {
    Block {
        header: Header {
            version: Version::from_consensus(1),
            prev_blockhash: BlockHash::all_zeros(),
            merkle_root: TxMerkleNode::all_zeros(),
            time: 0,
            bits: CompactTarget::from_consensus(0),
            nonce: 0,
        },
        txdata: txs,
    }
}

pub(crate) fn magic_bytes_strategy() -> impl Strategy<Value = [u8; 4]> {
    any::<[u8; 4]>()
}

pub(crate) fn build_da_blob(block_num: u64) -> DaBlob {
    DaBlob {
        update_seq_no: block_num,
        evm_header: build_evm_header(block_num),
        state_diff: Default::default(),
    }
}

pub(crate) fn build_multi_chunk_da_blob(
    block_num: u64,
    bytecode_len: usize,
    fill_byte: u8,
) -> DaBlob {
    let mut blob = build_da_blob(block_num);
    blob.state_diff
        .deployed_bytecodes
        .insert(Default::default(), vec![fill_byte; bytecode_len].into());
    blob
}

pub(crate) fn multi_chunk_bytecode_len_strategy() -> impl Strategy<Value = usize> {
    MIN_MULTI_CHUNK_BYTECODE_LEN..=MAX_MULTI_CHUNK_BYTECODE_LEN
}

pub(crate) fn build_parsed_envelope_from_chunk_bytes(chunks: Vec<Vec<u8>>) -> ParsedEnvelope {
    let txid = Txid::from_byte_array(derive_synthetic_txid_bytes(&chunks));
    ParsedEnvelope::new(txid, chunks)
}

fn build_evm_header(block_num: u64) -> EvmHeaderSummary {
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

pub(crate) fn make_deterministic_pubkey(seed: u8) -> XOnlyPublicKey {
    commit_reveal_fixtures::make_xonly_pubkey(seed)
}

fn derive_synthetic_txid_bytes(chunks: &[Vec<u8>]) -> [u8; 32] {
    let mut seed = Vec::new();
    for chunk in chunks {
        seed.extend_from_slice(&chunk.len().to_le_bytes());
        seed.extend_from_slice(chunk);
    }
    sha256::Hash::hash(&seed).to_byte_array()
}
