//! Main EE state diff types for DA encoding.

use std::collections::BTreeMap;

use revm_primitives::{Address, B256};
use serde::{Deserialize, Serialize};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::{
    account::{DaAccountChange, DaAccountChangeSerde},
    storage::DaAccountStorageDiff,
};

/// Complete EE state diff for a batch, using DA framework types.
#[derive(Clone, Debug, Default)]
pub struct DaEeStateDiff {
    /// Account changes, sorted by address for deterministic encoding.
    pub accounts: BTreeMap<Address, DaAccountChange>,
    /// Storage slot changes per account, sorted by address.
    pub storage: BTreeMap<Address, DaAccountStorageDiff>,
    /// Code hashes of deployed contracts (deduplicated).
    /// Full bytecode can be fetched from DB using these hashes.
    pub deployed_code_hashes: Vec<B256>,
}

/// Serde-friendly representation of DaEeStateDiff for RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaEeStateDiffSerde {
    /// Account changes, sorted by address.
    pub accounts: BTreeMap<Address, DaAccountChangeSerde>,
    /// Storage slot changes per account.
    pub storage: BTreeMap<Address, DaAccountStorageDiff>,
    /// Code hashes of deployed contracts.
    pub deployed_code_hashes: Vec<B256>,
}

impl From<&DaEeStateDiff> for DaEeStateDiffSerde {
    fn from(diff: &DaEeStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage.clone(),
            deployed_code_hashes: diff.deployed_code_hashes.clone(),
        }
    }
}

impl From<DaEeStateDiff> for DaEeStateDiffSerde {
    fn from(diff: DaEeStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage,
            deployed_code_hashes: diff.deployed_code_hashes,
        }
    }
}

impl From<DaEeStateDiffSerde> for DaEeStateDiff {
    fn from(serde: DaEeStateDiffSerde) -> Self {
        Self {
            accounts: serde
                .accounts
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            storage: serde.storage,
            deployed_code_hashes: serde.deployed_code_hashes,
        }
    }
}

impl DaEeStateDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the diff is empty.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.storage.is_empty() && self.deployed_code_hashes.is_empty()
    }

    /// Merges another diff into this one.
    ///
    /// Later changes override earlier ones. Used for batch aggregation.
    pub fn merge(&mut self, other: &DaEeStateDiff) {
        // Merge accounts - later changes override
        for (addr, change) in &other.accounts {
            self.accounts.insert(*addr, change.clone());
        }

        // Merge storage - later slot values override
        for (addr, other_storage) in &other.storage {
            let storage = self.storage.entry(*addr).or_default();
            for (key, value) in other_storage.iter() {
                if let Some(v) = value {
                    storage.set_slot(*key, *v);
                } else {
                    storage.delete_slot(*key);
                }
            }
        }

        // Merge deployed code hashes (deduplicate)
        for hash in &other.deployed_code_hashes {
            if !self.deployed_code_hashes.contains(hash) {
                self.deployed_code_hashes.push(*hash);
            }
        }
    }
}

impl Codec for DaEeStateDiff {
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
            let change = DaAccountChange::decode(dec)?;
            accounts.insert(addr, change);
        }

        // Decode storage
        let storage_count = u32::decode(dec)? as usize;
        let mut storage = BTreeMap::new();
        for _ in 0..storage_count {
            let mut addr_buf = [0u8; 20];
            dec.read_buf(&mut addr_buf)?;
            let addr = Address::from(addr_buf);
            let storage_diff = DaAccountStorageDiff::decode(dec)?;
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
    use crate::account::DaAccountDiff;

    #[test]
    fn test_ee_state_diff_roundtrip() {
        let mut diff = DaEeStateDiff::new();

        // Add account change
        diff.accounts.insert(
            Address::from([0x11u8; 20]),
            DaAccountChange::Created(DaAccountDiff::new_created(
                U256::from(1000),
                1,
                B256::from([0x22u8; 32]),
            )),
        );

        // Add storage change
        let mut storage = DaAccountStorageDiff::new();
        storage.set_slot(U256::from(1), U256::from(100));
        diff.storage.insert(Address::from([0x11u8; 20]), storage);

        // Add deployed code hash
        diff.deployed_code_hashes.push(B256::from([0x33u8; 32]));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: DaEeStateDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.accounts.len(), 1);
        assert_eq!(decoded.storage.len(), 1);
        assert_eq!(decoded.deployed_code_hashes.len(), 1);
        assert_eq!(decoded.deployed_code_hashes[0], B256::from([0x33u8; 32]));
    }

    #[test]
    fn test_empty_diff_size() {
        let diff = DaEeStateDiff::new();
        let encoded = encode_to_vec(&diff).unwrap();
        // Should be minimal: 3 u32 counts (0, 0, 0) = 3 bytes minimum
        assert!(encoded.len() <= 12);
    }
}
