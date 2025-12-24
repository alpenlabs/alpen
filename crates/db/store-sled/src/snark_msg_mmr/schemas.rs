use strata_db_types::mmr_helpers::MmrMetadata;
use strata_primitives::buf::Buf32;
use typed_sled::codec::{CodecError, KeyCodec};

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_value_codec};

define_table_with_integer_key!(
    /// MMR node storage: position -> hash
    (SnarkMsgMmrNodeSchema) u64 => Buf32
);

/// MMR metadata schema: singleton storage
#[derive(Clone, Copy, Debug, Default)]
pub struct SnarkMsgMmrMetaSchema;

impl typed_sled::Schema for SnarkMsgMmrMetaSchema {
    const TREE_NAME: typed_sled::schema::TreeName =
        typed_sled::schema::TreeName("SnarkMsgMmrMetaSchema");
    type Key = ();
    type Value = MmrMetadata;
}

impl SnarkMsgMmrMetaSchema {
    const fn tree_name() -> &'static str {
        "SnarkMsgMmrMetaSchema"
    }
}

impl_borsh_value_codec!(SnarkMsgMmrMetaSchema, MmrMetadata);

impl KeyCodec<SnarkMsgMmrMetaSchema> for () {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(vec![0u8])
    }

    fn decode_key(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() == 1 && bytes[0] == 0 {
            Ok(())
        } else {
            Err(CodecError::InvalidKeyLength {
                schema: "SnarkMsgMmrMetaSchema",
                expected: 1,
                actual: bytes.len(),
            })
        }
    }
}
