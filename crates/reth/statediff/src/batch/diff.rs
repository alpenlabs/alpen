//! Main batch state diff type for DA encoding.

use std::collections::BTreeMap;

use alloy_primitives::Bytes;
use revm_primitives::{Address, B256};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{decode_map_with, encode_map_with};

use super::{AccountChange, StorageDiff};
use crate::codec::{CodecAddress, CodecB256};

/// Complete state diff for a batch, optimized for DA encoding.
///
/// This is the type that gets posted to the DA layer. It represents
/// the net change over a range of blocks, with reverts already filtered out.
#[derive(Clone, Debug, Default)]
pub struct BatchStateDiff {
    /// Account changes, sorted by address for deterministic encoding.
    pub accounts: BTreeMap<Address, AccountChange>,
    /// Storage slot changes per account, sorted by address.
    pub storage: BTreeMap<Address, StorageDiff>,
    /// Deployed contract bytecodes keyed by code hash (deduplicated).
    /// Full bytecode is included for DA reconstruction without DB access.
    pub deployed_bytecodes: BTreeMap<B256, Bytes>,
}

impl BatchStateDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the diff is empty.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.storage.is_empty() && self.deployed_bytecodes.is_empty()
    }
}

/// Wrapper for Bytes that implements Codec with length-prefixed encoding.
#[derive(Clone, Debug)]
struct CodecBytes(Bytes);

impl Codec for CodecBytes {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode length as u32 varint, then raw bytes
        (self.0.len() as u32).encode(enc)?;
        enc.write_buf(&self.0)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u32::decode(dec)? as usize;
        let mut buf = vec![0u8; len];
        dec.read_buf(&mut buf)?;
        Ok(Self(Bytes::from(buf)))
    }
}

impl Codec for BatchStateDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        encode_map_with(&self.accounts, enc, |a| CodecAddress(*a), Clone::clone)?;
        encode_map_with(&self.storage, enc, |a| CodecAddress(*a), Clone::clone)?;
        // Encode bytecodes as map: hash -> bytes
        encode_map_with(
            &self.deployed_bytecodes,
            enc,
            |h| CodecB256(*h),
            |b| CodecBytes(b.clone()),
        )?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let accounts = decode_map_with(dec, |k: CodecAddress| k.0, |v| v)?;
        let storage = decode_map_with(dec, |k: CodecAddress| k.0, |v| v)?;
        let deployed_bytecodes = decode_map_with(dec, |k: CodecB256| k.0, |v: CodecBytes| v.0)?;

        Ok(Self {
            accounts,
            storage,
            deployed_bytecodes,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;
    use crate::batch::AccountDiff;

    #[test]
    fn test_batch_state_diff_roundtrip() {
        let mut diff = BatchStateDiff::new();

        // Add account change
        diff.accounts.insert(
            Address::from([0x11u8; 20]),
            AccountChange::Created(AccountDiff::new_created(
                U256::from(1000),
                1,
                B256::from([0x22u8; 32]),
            )),
        );

        // Add storage change
        let mut storage = StorageDiff::new();
        storage.set_slot(U256::from(1), U256::from(100));
        diff.storage.insert(Address::from([0x11u8; 20]), storage);

        // Add deployed bytecode
        let bytecode = Bytes::from_static(&[0x60, 0x80, 0x60, 0x40, 0x52]); // Sample EVM bytecode
        diff.deployed_bytecodes
            .insert(B256::from([0x33u8; 32]), bytecode.clone());

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: BatchStateDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.accounts.len(), 1);
        assert_eq!(decoded.storage.len(), 1);
        assert_eq!(decoded.deployed_bytecodes.len(), 1);
        assert_eq!(
            decoded
                .deployed_bytecodes
                .get(&B256::from([0x33u8; 32]))
                .unwrap(),
            &bytecode
        );
    }

    #[test]
    fn test_empty_diff_size() {
        let diff = BatchStateDiff::new();
        let encoded = encode_to_vec(&diff).unwrap();
        // Should be minimal: 3 u32 counts (0, 0, 0) = 3 bytes minimum
        assert!(encoded.len() <= 12);
    }

    #[test]
    fn test_bytecode_encoding_size() {
        let mut diff = BatchStateDiff::new();

        // Add a realistic contract bytecode (~1KB)
        let bytecode = Bytes::from(vec![0x60u8; 1024]);
        diff.deployed_bytecodes
            .insert(B256::from([0x11u8; 32]), bytecode);

        let encoded = encode_to_vec(&diff).unwrap();
        // Should include: 3 map counts + 32 byte hash + 4 byte length + 1024 bytes
        // Plus some overhead for map encoding
        assert!(encoded.len() > 1024);
        assert!(encoded.len() < 1100); // Not too much overhead
    }
}
