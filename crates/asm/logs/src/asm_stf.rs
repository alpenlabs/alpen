use ssz::{Decode, Encode};
use strata_asm_common::AsmLog;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_msg_fmt::TypeId;
use strata_predicate::PredicateKey;

use crate::constants::ASM_STF_UPDATE_LOG_TYPE;
pub use crate::ssz_generated::ssz::checkpoint::{AsmStfUpdate, AsmStfUpdateRef, PredicateBytes};

fn encode_predicate(new_predicate: PredicateKey) -> PredicateBytes {
    PredicateBytes::new(new_predicate.as_ssz_bytes())
        .expect("asm-stf predicate must stay within SSZ bounds")
}

fn decode_predicate(bytes: &[u8]) -> PredicateKey {
    PredicateKey::from_ssz_bytes(bytes).expect("asm-stf predicate bytes must remain valid")
}

/// Details for an execution environment verification key update.
impl AsmStfUpdate {
    /// Create a new AsmStfUpdate instance.
    pub fn new(new_predicate: PredicateKey) -> Self {
        Self {
            new_predicate: encode_predicate(new_predicate),
        }
    }

    pub fn new_predicate(&self) -> PredicateKey {
        decode_predicate(&self.new_predicate)
    }

    pub fn into_new_predicate(self) -> PredicateKey {
        decode_predicate(&self.new_predicate)
    }
}

impl Codec for AsmStfUpdate {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = Varint::decode(dec)?;
        let len_usize = len.inner() as usize;
        let mut buffer = vec![0u8; len_usize];
        dec.read_buf(&mut buffer)?;
        Self::from_ssz_bytes(&buffer).map_err(|_| CodecError::MalformedField("ssz"))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let bytes = self.as_ssz_bytes();
        let len = Varint::new_usize(bytes.len()).ok_or(CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(&bytes)
    }
}

impl AsmLog for AsmStfUpdate {
    const TY: TypeId = ASM_STF_UPDATE_LOG_TYPE;
}
