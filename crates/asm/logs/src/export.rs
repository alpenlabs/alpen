use ssz::{Decode, Encode};
use strata_asm_common::AsmLog;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_msg_fmt::TypeId;

use crate::constants::NEW_EXPORT_ENTRY_LOG_TYPE;
pub use crate::ssz_generated::ssz::checkpoint::{NewExportEntry, NewExportEntryRef};

/// Details for an export state update event.
impl NewExportEntry {
    /// Create a new NewExportEntry instance.
    pub fn new(container_id: u8, entry_data: [u8; 32]) -> Self {
        Self {
            container_id,
            entry_data: entry_data.into(),
        }
    }

    pub fn container_id(&self) -> u8 {
        self.container_id
    }

    pub fn entry_data(&self) -> [u8; 32] {
        self.entry_data
            .as_ref()
            .try_into()
            .expect("export entry data must remain 32 bytes")
    }
}

impl Codec for NewExportEntry {
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

impl AsmLog for NewExportEntry {
    const TY: TypeId = NEW_EXPORT_ENTRY_LOG_TYPE;
}
