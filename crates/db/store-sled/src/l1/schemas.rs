use ssz::{Decode, Encode};
use strata_asm_common::AsmManifest;
use strata_primitives::{L1Height, l1::L1BlockId};
use typed_sled::codec::{CodecError, ValueCodec};

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_key_codec};

define_table_without_codec!(
    /// A table to store L1 Block data (as ASM Manifest). Maps block id to manifest
    (L1BlockSchema) L1BlockId => AsmManifest
);
impl_borsh_key_codec!(L1BlockSchema, L1BlockId);

impl ValueCodec<L1BlockSchema> for AsmManifest {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: sled::IVec) -> Result<Self::Decoded, CodecError> {
        Self::from_ssz_bytes(data.as_ref()).map_err(|err| CodecError::DeserializationFailed {
            schema: L1BlockSchema::tree_name(),
            source: err.into(),
        })
    }
}

define_table_with_integer_key!(
    /// A table to store canonical view of L1 chain
    (L1CanonicalBlockSchema) L1Height => L1BlockId
);

define_table_with_integer_key!(
    /// A table to keep track of all added blocks
    (L1BlocksByHeightSchema) L1Height => Vec<L1BlockId>
);
