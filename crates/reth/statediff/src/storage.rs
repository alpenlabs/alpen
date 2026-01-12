//! Storage slot diff types for DA encoding.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Diff for storage slots of an account.
///
/// Uses a sorted map for deterministic encoding.
/// Each slot value is encoded as a register (full replacement).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaAccountStorageDiff {
    /// Changed storage slots: slot_key -> new_value (None = deleted/zeroed).
    slots: BTreeMap<U256, Option<U256>>,
}

impl DaAccountStorageDiff {
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

impl Codec for DaAccountStorageDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode count as varint (u32 should be enough)
        (self.slots.len() as u32).encode(enc)?;

        // Encode each slot (already sorted due to BTreeMap)
        for (key, value) in &self.slots {
            enc.write_buf(&key.to_le_bytes::<32>())?;
            match value {
                Some(v) => {
                    true.encode(enc)?;
                    enc.write_buf(&v.to_le_bytes::<32>())?;
                }
                None => {
                    false.encode(enc)?;
                }
            }
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let count = u32::decode(dec)? as usize;
        let mut slots = BTreeMap::new();

        for _ in 0..count {
            let mut key_buf = [0u8; 32];
            dec.read_buf(&mut key_buf)?;
            let key = U256::from_le_bytes(key_buf);

            let has_value = bool::decode(dec)?;
            let value = if has_value {
                let mut value_buf = [0u8; 32];
                dec.read_buf(&mut value_buf)?;
                Some(U256::from_le_bytes(value_buf))
            } else {
                None
            };

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
        let mut diff = DaAccountStorageDiff::new();
        diff.set_slot(U256::from(1), U256::from(100));
        diff.set_slot(U256::from(2), U256::from(200));
        diff.delete_slot(U256::from(3));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: DaAccountStorageDiff = decode_buf_exact(&encoded).unwrap();

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
