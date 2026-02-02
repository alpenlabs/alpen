use ssz::{Decode, Encode};
use strata_asm_common::AsmManifest;
use strata_primitives::l1::L1BlockId;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::{define_table_with_integer_key, define_table_without_codec};

define_table_without_codec!(
    /// A table to store L1 Block data (as ASM Manifest). Maps block id to manifest
    (L1BlockSchema) L1BlockId => AsmManifest
);

impl KeyCodec<L1BlockSchema> for L1BlockId {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        Self::from_ssz_bytes(data).map_err(|err| CodecError::SerializationFailed {
            schema: L1BlockSchema::tree_name(),
            source: format!("SSZ decode error: {err:?}").into(),
        })
    }
}

impl ValueCodec<L1BlockSchema> for AsmManifest {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Self::from_ssz_bytes(data).map_err(|err| CodecError::SerializationFailed {
            schema: L1BlockSchema::tree_name(),
            source: format!("SSZ decode error: {err:?}").into(),
        })
    }
}

define_table_with_integer_key!(
    /// A table to store canonical view of L1 chain
    (L1CanonicalBlockSchema) u64 => L1BlockId
);

define_table_with_integer_key!(
    /// A table to keep track of all added blocks
    (L1BlocksByHeightSchema) u64 => Vec<L1BlockId>
);
