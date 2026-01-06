//! Guest-side chain segment building.

use std::result::Result as StdResult;

use sha2::{Digest, Sha256};
use strata_ee_acct_types::{CommitBlockData, CommitChainSegment};
use strata_ee_chain_types::ExecBlockPackage;
use strata_primitives::buf::Buf32;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum GuestBuilderError {
    #[error("Block hash mismatch: expected {expected:?}, computed {computed:?}")]
    BlkHashMismatch { expected: Buf32, computed: Buf32 },
}

pub(crate) type Result<T> = StdResult<T, GuestBuilderError>;

/// Build CommitChainSegment from exec packages and raw block bodies.
pub(crate) fn build_commit_segments(
    exec_block_packages: &[ExecBlockPackage],
    raw_block_bodies: &[Vec<u8>],
) -> Result<Vec<CommitChainSegment>> {
    let block_data_list = exec_block_packages
        .iter()
        .zip(raw_block_bodies.iter())
        .map(|(package, raw_block_bytes)| {
            // Verify the cryptographic commitment
            let raw_block_hash = Buf32::from(<[u8; 32]>::from(Sha256::digest(raw_block_bytes)));
            let expected_hash = package.commitment().raw_block_encoded_hash();

            if raw_block_hash != expected_hash {
                return Err(GuestBuilderError::BlkHashMismatch {
                    expected: expected_hash,
                    computed: raw_block_hash,
                });
            }

            Ok(CommitBlockData::new(
                package.clone(),
                raw_block_bytes.clone(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    let segment = CommitChainSegment::new(block_data_list);
    Ok(vec![segment])
}
