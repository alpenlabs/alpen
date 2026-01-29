//! Main batch state diff type for DA encoding.

use std::collections::BTreeMap;

use revm_primitives::{Address, B256};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{decode_map_with, decode_vec_with, encode_map_with, encode_vec_with};

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
    /// Code hashes of deployed contracts (deduplicated).
    /// Full bytecode can be fetched from DB using these hashes.
    pub deployed_code_hashes: Vec<B256>,
}

impl BatchStateDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the diff is empty.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.storage.is_empty() && self.deployed_code_hashes.is_empty()
    }
}

impl Codec for BatchStateDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        encode_map_with(&self.accounts, enc, |a| CodecAddress(*a), Clone::clone)?;
        encode_map_with(&self.storage, enc, |a| CodecAddress(*a), Clone::clone)?;
        encode_vec_with(&self.deployed_code_hashes, enc, |h| CodecB256(*h))?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let accounts = decode_map_with(dec, |k: CodecAddress| k.0, |v| v)?;
        let storage = decode_map_with(dec, |k: CodecAddress| k.0, |v| v)?;
        let deployed_code_hashes = decode_vec_with(dec, |h: CodecB256| h.0)?;

        Ok(Self {
            accounts,
            storage,
            deployed_code_hashes,
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

        // Add deployed code hash
        diff.deployed_code_hashes.push(B256::from([0x33u8; 32]));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: BatchStateDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.accounts.len(), 1);
        assert_eq!(decoded.storage.len(), 1);
        assert_eq!(decoded.deployed_code_hashes.len(), 1);
        assert_eq!(decoded.deployed_code_hashes[0], B256::from([0x33u8; 32]));
    }

    #[test]
    fn test_empty_diff_size() {
        let diff = BatchStateDiff::new();
        let encoded = encode_to_vec(&diff).unwrap();
        // Should be minimal: 3 u32 counts (0, 0, 0) = 3 bytes minimum
        assert!(encoded.len() <= 12);
    }
}
