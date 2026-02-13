//! Storage slot diff types for DA encoding.

use std::collections::BTreeMap;

use alloy_primitives::U256;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::codec::TrimmedStorageValue;

/// Diff for storage slots of an account.
///
/// Uses a sorted map for deterministic encoding.
/// Each slot value is encoded as a register (full replacement).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct StorageDiff {
    /// Changed storage slots: slot_key -> new_value (None = deleted/zeroed).
    slots: BTreeMap<U256, Option<U256>>,
}

impl StorageDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a slot value.
    pub fn set_slot(&mut self, key: U256, value: U256) {
        if value.is_zero() {
            self.slots.insert(key, None);
        } else {
            self.slots.insert(key, Some(value));
        }
    }

    /// Marks a slot as deleted (zeroed).
    pub fn delete_slot(&mut self, key: U256) {
        self.slots.insert(key, None);
    }

    /// Returns true if no slot changes.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Returns the number of changed slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Iterates over slot changes.
    pub fn iter(&self) -> impl Iterator<Item = (&U256, &Option<U256>)> {
        self.slots.iter()
    }
}

impl Codec for StorageDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode count as varint (u32 should be enough)
        (self.slots.len() as u32).encode(enc)?;

        // Encode each slot (already sorted due to BTreeMap)
        //
        // Keys are encoded as fixed 32 bytes (big-endian) because:
        // - Most storage keys are keccak256 hashes (mapping slots) which are uniformly distributed
        //   and won't benefit from leading-zero trimming
        // - Trimming would add 1-byte overhead for hash-based keys (33 vs 32 bytes)
        // - Simple slot indices (0, 1, 2...) are relatively rare in practice
        //
        // Values use trimmed encoding because they vary widely:
        // - Booleans, counters, timestamps: 1-5 bytes (huge savings)
        // - Addresses: 20 bytes (34% savings)
        // - Balances: 8-16 bytes (50-75% savings)
        // - Only full hashes have 1-byte overhead, which is rare for values
        for (key, value) in &self.slots {
            enc.write_buf(&key.to_be_bytes::<32>())?;
            TrimmedStorageValue(*value).encode(enc)?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let count = u32::decode(dec)? as usize;
        let mut slots = BTreeMap::new();

        for _ in 0..count {
            let mut key_buf = [0u8; 32];
            dec.read_buf(&mut key_buf)?;
            let key = U256::from_be_bytes(key_buf);
            let value = TrimmedStorageValue::decode(dec)?.0;
            slots.insert(key, value);
        }

        Ok(Self { slots })
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_storage_diff_roundtrip() {
        let mut diff = StorageDiff::new();
        diff.set_slot(U256::from(1), U256::from(100));
        diff.set_slot(U256::from(2), U256::from(200));
        diff.delete_slot(U256::from(3));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: StorageDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.len(), 3);
        assert_eq!(
            decoded.slots.get(&U256::from(1)),
            Some(&Some(U256::from(100)))
        );
        assert_eq!(
            decoded.slots.get(&U256::from(2)),
            Some(&Some(U256::from(200)))
        );
        assert_eq!(decoded.slots.get(&U256::from(3)), Some(&None));
    }
}
