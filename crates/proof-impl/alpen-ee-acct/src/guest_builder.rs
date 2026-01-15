//! Guest-side chain segment building.

use std::result::Result as StdResult;

use sha2::{Digest, Sha256};
use strata_ee_acct_types::{CommitBlockData, CommitChainSegment};
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::Hash;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum GuestBuilderError {
    #[error("blkid mismatch (expected {0:?}, computed {1:?})")]
    BlkidMismatch(Hash, Hash),
}

pub(crate) type Result<T> = StdResult<T, GuestBuilderError>;

/// Build [`CommitChainSegment`] from exec packages and raw blocks.
pub(crate) fn build_commit_segments(
    exec_block_packages: &[ExecBlockPackage],
    raw_blocks: &[Vec<u8>],
) -> Result<Vec<CommitChainSegment>> {
    let block_data_list = exec_block_packages
        .iter()
        .zip(raw_blocks.iter())
        .map(|(package, raw_block_bytes)| {
            // Verify the cryptographic commitment
            let raw_block_hash = Hash::new(Sha256::digest(raw_block_bytes).into());
            let expected_hash = package.commitment().raw_block_encoded_hash();

            if raw_block_hash != expected_hash {
                return Err(GuestBuilderError::BlkidMismatch(
                    expected_hash,
                    raw_block_hash,
                ));
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
