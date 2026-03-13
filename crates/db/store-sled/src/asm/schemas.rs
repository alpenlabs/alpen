use ssz::{Decode, Encode};
use strata_asm_common::{AnchorState, AsmLogEntry, AuxData};
use strata_primitives::l1::L1BlockCommitment;
use typed_sled::codec::{CodecError, ValueCodec};

use crate::{define_table_without_codec, impl_bincode_key_codec};

// ASM state per block schema and corresponding codecs implementation.
define_table_without_codec!(
    /// A table to store ASM state per l1 block.
    (AsmStateSchema) L1BlockCommitment => AnchorState
);
impl_bincode_key_codec!(AsmStateSchema, L1BlockCommitment);

impl ValueCodec<AsmStateSchema> for AnchorState {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: sled::IVec) -> Result<Self::Decoded, CodecError> {
        Self::from_ssz_bytes(data.as_ref()).map_err(|err| CodecError::DeserializationFailed {
            schema: AsmStateSchema::tree_name(),
            source: err.into(),
        })
    }
}

// ASM logs per block schema and corresponding codecs implementation.
define_table_without_codec!(
    /// A table to store ASM logs per l1 block.
    (AsmLogSchema) L1BlockCommitment => Vec<AsmLogEntry>
);
impl_bincode_key_codec!(AsmLogSchema, L1BlockCommitment);

impl ValueCodec<AsmLogSchema> for Vec<AsmLogEntry> {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: sled::IVec) -> Result<Self::Decoded, CodecError> {
        Self::from_ssz_bytes(data.as_ref()).map_err(|err| CodecError::DeserializationFailed {
            schema: AsmLogSchema::tree_name(),
            source: err.into(),
        })
    }
}

// ASM auxiliary data per block schema and corresponding codecs implementation.
define_table_without_codec!(
    /// A table to store ASM auxiliary data per l1 block.
    (AsmAuxDataSchema) L1BlockCommitment => AuxData
);
impl_bincode_key_codec!(AsmAuxDataSchema, L1BlockCommitment);

impl ValueCodec<AsmAuxDataSchema> for AuxData {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: sled::IVec) -> Result<Self::Decoded, CodecError> {
        Self::from_ssz_bytes(data.as_ref()).map_err(|err| CodecError::DeserializationFailed {
            schema: AsmAuxDataSchema::tree_name(),
            source: err.into(),
        })
    }
}
