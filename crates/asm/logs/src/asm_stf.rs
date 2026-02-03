use strata_asm_common::AsmLog;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_msg_fmt::TypeId;
use strata_predicate::{PredicateKey, PredicateKeyBuf};

use crate::constants::ASM_STF_UPDATE_LOG_TYPE;

/// Details for an execution environment verification key update.
#[derive(Debug, Clone, Codec)]
pub struct AsmStfUpdate {
    /// New execution environment state transition function verification key.
    new_predicate: CodecPredicateKey,
}

/// Serializer for [`PredicateKey`] as bytes for codec.
#[derive(Debug, Clone, Eq, PartialEq)]
struct CodecPredicateKey(PredicateKey);

impl CodecPredicateKey {
    fn new(inner: PredicateKey) -> Self {
        Self(inner)
    }

    fn inner(&self) -> &PredicateKey {
        &self.0
    }

    fn into_inner(self) -> PredicateKey {
        self.0
    }
}

impl Codec for CodecPredicateKey {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = Varint::decode(dec)?;
        let len_usize = len.inner() as usize;

        let mut buffer = vec![0u8; len_usize];
        dec.read_buf(&mut buffer)?;

        let key = PredicateKeyBuf::try_from(buffer.as_slice())
            .map_err(|_| CodecError::MalformedField("predicate_key"))?
            .to_owned();

        Ok(Self(key))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let bytes = self.0.as_buf_ref().to_bytes();
        let len = Varint::new_usize(bytes.len()).ok_or(CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(&bytes)?;

        Ok(())
    }
}

impl AsmStfUpdate {
    /// Create a new AsmStfUpdate instance.
    pub fn new(new_predicate: PredicateKey) -> Self {
        Self {
            new_predicate: CodecPredicateKey::new(new_predicate),
        }
    }

    pub fn new_predicate(&self) -> &PredicateKey {
        self.new_predicate.inner()
    }

    pub fn into_new_predicate(self) -> PredicateKey {
        self.new_predicate.into_inner()
    }
}

impl AsmLog for AsmStfUpdate {
    const TY: TypeId = ASM_STF_UPDATE_LOG_TYPE;
}
