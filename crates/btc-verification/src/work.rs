use std::{io, ops::AddAssign};

use bitcoin::Work;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use tree_hash::{PackedEncoding, TreeHash, TreeHashDigest, TreeHashType};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtcWork(Work);

impl Default for BtcWork {
    fn default() -> Self {
        Self(Work::from_le_bytes([0u8; 32]))
    }
}

impl From<Work> for BtcWork {
    fn from(work: Work) -> Self {
        Self(work)
    }
}

impl AddAssign for BtcWork {
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0 + rhs.0;
    }
}

impl BorshSerialize for BtcWork {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.0.to_le_bytes(), writer)
    }
}

impl BorshDeserialize for BtcWork {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let bytes = <[u8; 32]>::deserialize_reader(reader)?;
        Ok(Self(Work::from_le_bytes(bytes)))
    }
}

impl<'a> arbitrary::Arbitrary<'a> for BtcWork {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let bytes = <[u8; 32]>::arbitrary(u)?;
        Ok(Self(Work::from_le_bytes(bytes)))
    }
}

impl Encode for BtcWork {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        <[u8; 32] as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.0.to_le_bytes().ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        <Self as Encode>::ssz_fixed_len()
    }
}

impl Decode for BtcWork {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        <[u8; 32] as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let inner = <[u8; 32]>::from_ssz_bytes(bytes)?;
        Ok(Self(Work::from_le_bytes(inner)))
    }
}

impl<H: TreeHashDigest> TreeHash<H> for BtcWork {
    fn tree_hash_type() -> TreeHashType {
        <[u8; 32] as TreeHash<H>>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        <[u8; 32] as TreeHash<H>>::tree_hash_packed_encoding(&self.0.to_le_bytes())
    }

    fn tree_hash_packing_factor() -> usize {
        <[u8; 32] as TreeHash<H>>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> H::Output {
        <[u8; 32] as TreeHash<H>>::tree_hash_root(&self.0.to_le_bytes())
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use tree_hash::{Sha256Hasher, TreeHash};

    use super::*;

    #[test]
    fn test_ssz_roundtrip() {
        let work = BtcWork::from(Work::from_le_bytes([0xAB; 32]));

        let bytes = work.as_ssz_bytes();
        let decoded = BtcWork::from_ssz_bytes(&bytes).unwrap();

        assert_eq!(work, decoded);
    }

    #[test]
    fn test_tree_hash_deterministic() {
        let work = BtcWork::from(Work::from_le_bytes([0x11; 32]));

        let hash1 = <BtcWork as TreeHash<Sha256Hasher>>::tree_hash_root(&work);
        let hash2 = <BtcWork as TreeHash<Sha256Hasher>>::tree_hash_root(&work);

        assert_eq!(hash1, hash2);
    }
}
