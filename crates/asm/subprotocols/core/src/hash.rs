//! Hash utilities for the Core ASM subprotocol

use sha2::{Digest, Sha256};
use strata_primitives::buf::Buf32;

/// Computes a SHA256 hash of the given data
pub(crate) fn hash_data(data: &[u8]) -> Buf32 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let arr: [u8; 32] = result.into();
    Buf32::from(arr)
}
