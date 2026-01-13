//! Main batch state diff type for DA encoding.

use std::collections::BTreeMap;

use revm_primitives::{Address, B256};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use super::{AccountChange, StorageDiff};

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
        // Encode accounts (sorted by BTreeMap)
        (self.accounts.len() as u32).encode(enc)?;
        for (addr, change) in &self.accounts {
            enc.write_buf(addr.as_slice())?;
            change.encode(enc)?;
        }

        // Encode storage (sorted by BTreeMap)
        (self.storage.len() as u32).encode(enc)?;
        for (addr, storage_diff) in &self.storage {
            enc.write_buf(addr.as_slice())?;
            storage_diff.encode(enc)?;
        }

        // Encode deployed code hashes
        (self.deployed_code_hashes.len() as u32).encode(enc)?;
        for hash in &self.deployed_code_hashes {
            enc.write_buf(hash.as_slice())?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode accounts
        let accounts_count = u32::decode(dec)? as usize;
        let mut accounts = BTreeMap::new();
        for _ in 0..accounts_count {
            let mut addr_buf = [0u8; 20];
            dec.read_buf(&mut addr_buf)?;
            let addr = Address::from(addr_buf);
            let change = AccountChange::decode(dec)?;
            accounts.insert(addr, change);
        }

        // Decode storage
        let storage_count = u32::decode(dec)? as usize;
        let mut storage = BTreeMap::new();
        for _ in 0..storage_count {
            let mut addr_buf = [0u8; 20];
            dec.read_buf(&mut addr_buf)?;
            let addr = Address::from(addr_buf);
            let storage_diff = StorageDiff::decode(dec)?;
            storage.insert(addr, storage_diff);
        }

        // Decode deployed code hashes
        let code_count = u32::decode(dec)? as usize;
        let mut deployed_code_hashes = Vec::with_capacity(code_count);
        for _ in 0..code_count {
            let mut hash_buf = [0u8; 32];
            dec.read_buf(&mut hash_buf)?;
            deployed_code_hashes.push(B256::from(hash_buf));
        }

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
