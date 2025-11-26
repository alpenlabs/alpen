//! EVM write batch implementation.

use reth_trie::HashedPostState;
use revm_primitives::alloy_primitives::Bloom;
use strata_codec::{Codec, CodecError};

use super::Hash;
use crate::codec_shims::{decode_hashed_post_state, encode_hashed_post_state};

/// Write batch for EVM execution containing state changes.
///
/// This wraps Reth's HashedPostState which contains the differences (deltas)
/// in account states and storage slots after executing a block. It's used to
/// apply state changes to the sparse EthereumState.
///
/// Also stores execution metadata (state root from header intrinsics, logs bloom)
/// needed for block header completion and state root verification during merge.
#[derive(Clone, Debug)]
pub struct EvmWriteBatch {
    hashed_post_state: HashedPostState,
    /// The state root extracted from block header intrinsics.
    ///
    /// This value is taken directly from `header_intrinsics.state_root` during
    /// block execution, NOT computed from the pre-state. Actual verification
    /// occurs in `merge_write_into_state` after the state is mutated, where
    /// we compute the real state root and compare it against this value.
    /// This approach avoids an expensive state clone in zkVM.
    intrinsics_state_root: Hash,
    /// The accumulated logs bloom from all receipts
    logs_bloom: Bloom,
}

impl EvmWriteBatch {
    /// Creates a new EvmWriteBatch from a HashedPostState and header intrinsics metadata.
    pub fn new(
        hashed_post_state: HashedPostState,
        intrinsics_state_root: Hash,
        logs_bloom: Bloom,
    ) -> Self {
        Self {
            hashed_post_state,
            intrinsics_state_root,
            logs_bloom,
        }
    }

    /// Gets a reference to the underlying HashedPostState.
    pub fn hashed_post_state(&self) -> &HashedPostState {
        &self.hashed_post_state
    }

    /// Gets the state root from block header intrinsics.
    ///
    /// This value is verified against the actual computed state root
    /// during `merge_write_into_state`.
    pub fn intrinsics_state_root(&self) -> Hash {
        self.intrinsics_state_root
    }

    /// Gets the accumulated logs bloom.
    pub fn logs_bloom(&self) -> Bloom {
        self.logs_bloom
    }

    /// Consumes self and returns the underlying HashedPostState.
    pub fn into_hashed_post_state(self) -> HashedPostState {
        self.hashed_post_state
    }
}

impl Codec for EvmWriteBatch {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode HashedPostState using custom deterministic encoding
        encode_hashed_post_state(&self.hashed_post_state, enc)?;

        // Encode intrinsics_state_root (32 bytes)
        enc.write_buf(&self.intrinsics_state_root)?;

        // Encode logs_bloom (256 bytes)
        enc.write_buf(self.logs_bloom.as_slice())?;

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode HashedPostState using custom deterministic decoding
        let hashed_post_state = decode_hashed_post_state(dec)?;

        // Decode intrinsics_state_root (32 bytes)
        let mut intrinsics_state_root = [0u8; 32];
        dec.read_buf(&mut intrinsics_state_root)?;

        // Decode logs_bloom (256 bytes)
        let mut logs_bloom_bytes = [0u8; 256];
        dec.read_buf(&mut logs_bloom_bytes)?;
        let logs_bloom = Bloom::from(logs_bloom_bytes);

        Ok(Self {
            hashed_post_state,
            intrinsics_state_root,
            logs_bloom,
        })
    }
}
