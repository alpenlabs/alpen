//! Guest-side block deserialization and chain segment building
//!
//! This module handles:
//! - Deserializing blocks from host: [exec_block_package (SSZ)][raw_block_body (codec)]
//! - Building CommitBlockData with verified commitments
//! - Assembling CommitChainSegment from multiple blocks

use sha2::{Digest, Sha256};
use ssz::{Decode, Encode};
use strata_ee_acct_types::{CommitBlockData, CommitChainSegment};
use strata_ee_chain_types::ExecBlockPackage;
use strata_primitives::buf::Buf32;

#[derive(Debug, thiserror::Error)]
pub(crate) enum GuestBuilderError {
    #[error("SSZ decode error")]
    SszDecode,
    #[expect(dead_code, reason = "Reserved for future codec decoding errors")]
    #[error("Codec decode error: {0}")]
    CodecDecode(String),
    #[error("Block hash mismatch: expected {expected:?}, computed {computed:?}")]
    HashMismatch { expected: Buf32, computed: Buf32 },
}

pub(crate) type Result<T> = std::result::Result<T, GuestBuilderError>;

/// Deserialize and build CommitBlockData from host-provided bytes (GUEST-SIDE)
///
/// The bytes format is: `[exec_block_package (SSZ)][raw_block_body (strata_codec)]`
///
/// This function:
/// 1. Deserializes ExecBlockPackage from SSZ
/// 2. Calculates where the package ends
/// 3. Extracts the raw block bytes
/// 4. Verifies the hash of raw block bytes matches the commitment
/// 5. Returns CommitBlockData
///
/// Note: We only verify the raw_block_encoded_hash (the cryptographic commitment).
///
/// # Arguments
/// * `bytes` - Concatenated bytes from host: [exec_block_package][raw_block_body]
///
/// # Returns
/// CommitBlockData ready to be added to CommitChainSegment
fn deserialize_and_build_block_data(bytes: &[u8]) -> Result<CommitBlockData> {
    // Decode ExecBlockPackage from SSZ
    let package =
        ExecBlockPackage::from_ssz_bytes(bytes).map_err(|_| GuestBuilderError::SszDecode)?;

    // Calculate how many bytes the SSZ encoding used
    let package_ssz = package.as_ssz_bytes();
    let package_len = package_ssz.len();

    // The rest is the raw block body
    let raw_block = &bytes[package_len..];

    // Verify the cryptographic commitment: hash of raw block bytes
    let raw_block_hash = Buf32::from(<[u8; 32]>::from(Sha256::digest(raw_block)));
    let expected_hash = package.commitment().raw_block_encoded_hash();

    if raw_block_hash != expected_hash {
        return Err(GuestBuilderError::HashMismatch {
            expected: expected_hash,
            computed: raw_block_hash,
        });
    }

    // Create CommitBlockData with the verified package and raw block
    Ok(CommitBlockData::new(package, raw_block.to_vec()))
}

/// Build CommitChainSegment(s) from host-provided block data (GUEST-SIDE)
///
/// This is the main entry point for processing blocks from the host.
///
/// # Arguments
/// * `block_data_bytes` - Vec of serialized blocks, each in format
///   [exec_block_package][raw_block_body]
///
/// # Returns
/// Vec<CommitChainSegment> - Currently returns one segment containing all blocks
pub(crate) fn build_commit_segments_from_blocks(
    block_data_bytes: Vec<Vec<u8>>,
) -> Result<Vec<CommitChainSegment>> {
    let mut block_data_list = Vec::new();

    // Deserialize and verify each block
    for bytes in block_data_bytes {
        let block_data = deserialize_and_build_block_data(&bytes)?;
        block_data_list.push(block_data);
    }

    // Build one CommitChainSegment from all blocks
    let segment = CommitChainSegment::new(block_data_list);

    // Return as Vec for compatibility with existing code
    Ok(vec![segment])
}
