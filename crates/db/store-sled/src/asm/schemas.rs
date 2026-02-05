use ssz::{Decode, Encode};
use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_primitives::l1::L1BlockCommitment;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::{
    define_table_with_seek_key_codec, define_table_without_codec,
    lexicographic::{decode_key, encode_key},
};

// ASM state per block schema and corresponding codecs implementation.
define_table_with_seek_key_codec!(
    /// A table to store ASM state per l1 block.
    (AsmStateSchema) L1BlockCommitment => AnchorState
);

// ASM logs per block schema and corresponding codecs implementation.
define_table_without_codec!(
    /// A table to store ASM logs per l1 block.
    (AsmLogSchema) L1BlockCommitment => Vec<AsmLogEntry>
);

impl KeyCodec<AsmLogSchema> for L1BlockCommitment {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(encode_key(self))
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        decode_key(data).map_err(|err| CodecError::SerializationFailed {
            schema: AsmLogSchema::tree_name(),
            source: err.into(),
        })
    }
}

impl ValueCodec<AsmLogSchema> for Vec<AsmLogEntry> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Self::from_ssz_bytes(data).map_err(|err| CodecError::SerializationFailed {
            schema: AsmLogSchema::tree_name(),
            source: format!("SSZ decode error: {err:?}").into(),
        })
    }
}
