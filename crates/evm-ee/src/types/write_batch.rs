//! EVM write batch implementation.

use reth_trie::HashedPostState;
use strata_codec::{Codec, CodecError};

use crate::codec_shims::{decode_hashed_post_state, encode_hashed_post_state};

/// Write batch for EVM execution containing state changes.
///
/// This wraps Reth's HashedPostState which contains the differences (deltas)
/// in account states and storage slots after executing a block. It's used to
/// apply state changes to the sparse EthereumState.
#[derive(Clone, Debug)]
pub struct EvmWriteBatch {
    hashed_post_state: HashedPostState,
}

impl EvmWriteBatch {
    /// Creates a new EvmWriteBatch from execution state changes.
    pub fn new(hashed_post_state: HashedPostState) -> Self {
        Self { hashed_post_state }
    }

    /// Gets a reference to the underlying HashedPostState.
    pub fn hashed_post_state(&self) -> &HashedPostState {
        &self.hashed_post_state
    }

    /// Consumes self and returns the underlying HashedPostState.
    pub fn into_hashed_post_state(self) -> HashedPostState {
        self.hashed_post_state
    }
}

impl Codec for EvmWriteBatch {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        encode_hashed_post_state(&self.hashed_post_state, enc)
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let hashed_post_state = decode_hashed_post_state(dec)?;
        Ok(Self { hashed_post_state })
    }
}
