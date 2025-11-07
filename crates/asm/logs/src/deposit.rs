use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_codec::{BufDecoder, Codec, CodecError, Decoder, Encoder};
use strata_msg_fmt::TypeId;

use crate::constants::LogTypeId;

/// Details for a deposit operation.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct DepositLog {
    /// Identifier of the target execution environment.
    pub ee_id: u32,
    /// Amount in satoshis.
    pub amount: u64,
    /// Serialized address for the operation.
    pub addr: Vec<u8>,
}

impl DepositLog {
    /// Create a new DepositLog instance.
    pub fn new(ee_id: u32, amount: u64, addr: Vec<u8>) -> Self {
        Self {
            ee_id,
            amount,
            addr,
        }
    }

    /// Converts to bytes using strata encoder.
    pub fn to_raw_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode(&mut buf)
            .expect("should be able to encode DepositLog to bytes");
        buf
    }

    /// Tries from raw_bytes.
    pub fn try_from_raw_bytes(buf: Vec<u8>) -> Result<Self, CodecError> {
        let mut dec = BufDecoder::new(buf);
        Self::decode(&mut dec)
    }
}

impl AsmLog for DepositLog {
    const TY: TypeId = LogTypeId::Deposit as u16;
}

impl Codec for DepositLog {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.ee_id.encode(enc)?;
        self.amount.encode(enc)?;

        let len = self.addr.len() as u64;
        len.encode(enc)?;

        enc.write_buf(&self.addr)?;

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let ee_id = u32::decode(dec)?;
        let amount = u64::decode(dec)?;
        let len = u64::decode(dec)?;

        let mut addr = vec![0u8; len as usize];
        dec.read_buf(&mut addr)?;

        Ok(Self::new(ee_id, amount, addr))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{BufDecoder, Codec};

    use super::DepositLog;

    proptest! {
        #[test]
        fn test_dep_log_codec_roundtrip(
            ee_id in 0u32..u32::MAX,
            amount in 0u64..u64::MAX,
            addr in prop::collection::vec(any::<u8>(), 0..1000)
        ) {
            let log = DepositLog::new(ee_id, amount, addr);
            let mut buf = vec![];

            log .encode(&mut buf).unwrap();

            let mut dec = BufDecoder::new(buf);
            let decoded = DepositLog::decode(&mut dec).unwrap();
            assert_eq!(log, decoded);
        }
    }
}
