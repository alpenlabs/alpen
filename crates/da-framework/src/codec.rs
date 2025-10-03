//! Codec re-exports from strata-codec.

// Re-export everything from strata-codec
pub use strata_codec::{Codec, CodecError, Decoder, Encoder, decode_buf_exact, encode_to_vec};

// Create type alias for Result
pub type CodecResult<T> = Result<T, CodecError>;
