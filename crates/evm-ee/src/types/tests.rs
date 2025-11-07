//! Tests for EVM types Codec implementations.

use alloy_consensus::Header;
use revm_primitives::alloy_primitives::{Address, B256, Bloom, Bytes, U256};
use rsp_client_executor::io::EthClientExecutorInput;
use serde::Deserialize;
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_ee_acct_types::ExecHeader;

use super::{EvmBlock, EvmBlockBody, EvmHeader, EvmPartialState};

#[derive(Deserialize)]
struct TestData {
    witness: EthClientExecutorInput,
}

/// Helper function to load witness test data from the reference implementation.
fn load_witness_test_data() -> EthClientExecutorInput {
    let test_data_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("proof-impl/evm-ee-stf/test_data/witness_params.json");

    let json_content = std::fs::read_to_string(&test_data_path)
        .expect("Failed to read witness_params.json - make sure reference crate exists");

    let test_data: TestData =
        serde_json::from_str(&json_content).expect("Failed to parse test data");

    test_data.witness
}

/// Helper function to create a test header with realistic values
fn create_test_header() -> Header {
    use revm_primitives::alloy_primitives::B64;

    Header {
        parent_hash: B256::from([1u8; 32]),
        ommers_hash: B256::from([2u8; 32]),
        beneficiary: Address::from([3u8; 20]),
        state_root: B256::from([4u8; 32]),
        transactions_root: B256::from([5u8; 32]),
        receipts_root: B256::from([6u8; 32]),
        logs_bloom: Bloom::ZERO,
        difficulty: U256::from(12345u64),
        number: 1000u64,
        gas_limit: 30_000_000u64,
        gas_used: 21_000u64,
        timestamp: 1234567890u64,
        extra_data: Bytes::from(vec![7u8, 8u8, 9u8]),
        mix_hash: B256::from([10u8; 32]),
        nonce: B64::from([0, 0, 0, 0, 0, 0, 0, 42]),
        base_fee_per_gas: Some(1_000_000_000u64),
        withdrawals_root: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        parent_beacon_block_root: None,
        requests_hash: None,
    }
}

#[test]
fn test_evm_header_codec_roundtrip() {
    let header = create_test_header();
    let evm_header = EvmHeader::new(header.clone());

    // Encode
    let encoded = encode_to_vec(&evm_header).expect("encode failed");

    // Decode
    let decoded: EvmHeader = decode_buf_exact(&encoded).expect("decode failed");

    // Verify round-trip
    assert_eq!(decoded.header(), &header);
}

#[test]
fn test_evm_header_codec_with_post_merge_fields() {
    let mut header = create_test_header();
    // Add post-merge (Shanghai/Cancun) fields
    header.withdrawals_root = Some(B256::from([11u8; 32]));
    header.blob_gas_used = Some(131072u64);
    header.excess_blob_gas = Some(0u64);
    header.parent_beacon_block_root = Some(B256::from([12u8; 32]));

    let evm_header = EvmHeader::new(header.clone());

    // Encode and decode
    let encoded = encode_to_vec(&evm_header).expect("encode failed");
    let decoded: EvmHeader = decode_buf_exact(&encoded).expect("decode failed");

    // Verify all optional fields preserved
    assert_eq!(decoded.header(), &header);
}

#[test]
fn test_evm_header_exec_header_trait() {
    let header = create_test_header();
    let evm_header = EvmHeader::new(header.clone());

    // Test ExecHeader trait methods
    assert_eq!(evm_header.get_state_root(), header.state_root.0);
    assert_eq!(evm_header.compute_block_id(), header.hash_slow().0);
    assert_eq!(evm_header.get_intrinsics().number, header.number);
    assert_eq!(evm_header.block_number(), header.number);
}

#[test]
fn test_evm_block_body_codec_empty() {
    // Create an empty block body (no transactions, no withdrawals)
    let body = EvmBlockBody::new(vec![]);

    // Encode
    let encoded = encode_to_vec(&body).expect("encode failed");

    // Decode
    let decoded: EvmBlockBody = decode_buf_exact(&encoded).expect("decode failed");

    // Verify empty
    assert_eq!(decoded.transaction_count(), 0);
    assert!(decoded.transactions().is_empty());
    assert!(decoded.body().withdrawals.is_none());
}

#[test]
fn test_evm_block_body_codec_roundtrip() {
    // Load witness data and extract block body
    let witness = load_witness_test_data();

    use reth_primitives_traits::Block;
    let block_body = witness.current_block.body().clone();
    let body = EvmBlockBody::from_alloy_body(block_body.clone());

    // Encode
    let encoded = encode_to_vec(&body).expect("encode failed");

    // Decode
    let decoded: EvmBlockBody = decode_buf_exact(&encoded).expect("decode failed");

    // Verify the entire body matches (compares all transactions and withdrawals)
    assert_eq!(
        decoded.body(),
        body.body(),
        "Block body should match exactly"
    );
}

#[test]
fn test_evm_block_codec_roundtrip() {
    // Load witness data and construct block
    let witness = load_witness_test_data();

    use reth_primitives_traits::Block;
    let header = witness.current_block.header().clone();
    let evm_header = EvmHeader::new(header.clone());

    let block_body = witness.current_block.body().clone();
    let evm_body = EvmBlockBody::from_alloy_body(block_body);

    let block = EvmBlock::new(evm_header, evm_body);

    // Encode
    let encoded = encode_to_vec(&block).expect("encode failed");

    // Decode
    let decoded: EvmBlock = decode_buf_exact(&encoded).expect("decode failed");

    // Verify header matches
    assert_eq!(
        decoded.header().header(),
        block.header().header(),
        "Header should match exactly"
    );

    // Verify body matches (compares all transactions and withdrawals)
    assert_eq!(
        decoded.body().body(),
        block.body().body(),
        "Block body should match exactly"
    );
}

#[test]
fn test_evm_partial_state_codec_roundtrip() {
    let witness = load_witness_test_data();
    let partial_state = EvmPartialState::new(
        witness.parent_state,
        witness.bytecodes,
        witness.ancestor_headers,
    );

    let encoded = encode_to_vec(&partial_state).expect("encode failed");
    let decoded: EvmPartialState = decode_buf_exact(&encoded).expect("decode failed");

    // Verify state root matches
    assert_eq!(
        decoded.ethereum_state().state_root(),
        partial_state.ethereum_state().state_root()
    );

    // Verify bytecode hashes match
    let original_hashes: Vec<_> = partial_state.bytecodes().iter().map(|b| b.hash_slow()).collect();
    let decoded_hashes: Vec<_> = decoded.bytecodes().iter().map(|b| b.hash_slow()).collect();
    assert_eq!(decoded_hashes, original_hashes);

    // Verify ancestor headers match
    assert_eq!(decoded.ancestor_headers(), partial_state.ancestor_headers());
}

#[test]
fn test_evm_write_batch_codec_roundtrip() {
    use reth_trie::HashedPostState;
    use revm_primitives::alloy_primitives::Bloom;

    use super::EvmWriteBatch;

    // Create a simple write batch with default values
    let hashed_post_state = HashedPostState::default();
    let state_root = [42u8; 32];
    let logs_bloom = Bloom::from([0xAB; 256]);

    let write_batch = EvmWriteBatch::new(hashed_post_state, state_root, logs_bloom);

    // Encode
    let encoded = encode_to_vec(&write_batch).expect("encode failed");

    // Decode
    let decoded: EvmWriteBatch = decode_buf_exact(&encoded).expect("decode failed");

    // Verify state_root matches
    assert_eq!(decoded.state_root(), write_batch.state_root());

    // Verify logs_bloom matches
    assert_eq!(decoded.logs_bloom(), write_batch.logs_bloom());
}
