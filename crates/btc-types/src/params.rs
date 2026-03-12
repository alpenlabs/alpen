use std::io;

use bitcoin::params::{MAINNET, Params};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use tree_hash::{PackedEncoding, TreeHash, TreeHashDigest, TreeHashType};

#[derive(Debug, Clone)]
pub struct BtcParams(Params);

impl PartialEq for BtcParams {
    fn eq(&self, other: &Self) -> bool {
        // Just compare the network since all other params derive from it
        self.0.network == other.0.network
    }
}

impl Eq for BtcParams {}

impl Default for BtcParams {
    fn default() -> Self {
        BtcParams(MAINNET.clone())
    }
}

impl BorshSerialize for BtcParams {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        let network_index = self.network_index()?;
        BorshSerialize::serialize(&network_index, writer)
    }
}

impl BorshDeserialize for BtcParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let network_index = u8::deserialize_reader(reader)?;
        let network = match network_index {
            0 => bitcoin::Network::Bitcoin,
            1 => bitcoin::Network::Testnet,
            2 => bitcoin::Network::Signet,
            3 => bitcoin::Network::Regtest,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid network index",
                ));
            }
        };
        Ok(BtcParams::from(Params::from(network)))
    }
}

impl Serialize for BtcParams {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Just serialize the network - the rest can be derived from it
        self.0.network.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BtcParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let network = bitcoin::Network::deserialize(deserializer)?;
        Ok(BtcParams::from(Params::from(network)))
    }
}

impl<'a> arbitrary::Arbitrary<'a> for BtcParams {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let networks = [
            bitcoin::Network::Bitcoin,
            bitcoin::Network::Testnet,
            bitcoin::Network::Signet,
            bitcoin::Network::Regtest,
        ];
        let network = u.choose(&networks)?;
        Ok(BtcParams::from(Params::from(*network)))
    }
}

impl From<Params> for BtcParams {
    fn from(params: Params) -> Self {
        BtcParams(params)
    }
}

impl BtcParams {
    fn network_index(&self) -> io::Result<u8> {
        match self.0.network {
            bitcoin::Network::Bitcoin => Ok(0),
            bitcoin::Network::Testnet => Ok(1),
            bitcoin::Network::Signet => Ok(2),
            bitcoin::Network::Regtest => Ok(3),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported network type",
            )),
        }
    }

    fn from_network_index(network_index: u8) -> io::Result<Self> {
        let network = match network_index {
            0 => bitcoin::Network::Bitcoin,
            1 => bitcoin::Network::Testnet,
            2 => bitcoin::Network::Signet,
            3 => bitcoin::Network::Regtest,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid network index",
                ));
            }
        };
        Ok(BtcParams::from(Params::from(network)))
    }

    pub fn into_inner(self) -> Params {
        self.0
    }

    pub fn inner(&self) -> &Params {
        &self.0
    }

    pub fn difficulty_adjustment_interval(&self) -> u64 {
        self.0.difficulty_adjustment_interval()
    }
}

impl Encode for BtcParams {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.network_index()
            .expect("btc params should only contain supported networks")
            .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        <Self as Encode>::ssz_fixed_len()
    }
}

impl Decode for BtcParams {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let network_index = u8::from_ssz_bytes(bytes)?;
        Self::from_network_index(network_index)
            .map_err(|err| DecodeError::BytesInvalid(err.to_string()))
    }
}

impl<H: TreeHashDigest> TreeHash<H> for BtcParams {
    fn tree_hash_type() -> TreeHashType {
        <u8 as TreeHash<H>>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        <u8 as TreeHash<H>>::tree_hash_packed_encoding(
            &self
                .network_index()
                .expect("btc params should only contain supported networks"),
        )
    }

    fn tree_hash_packing_factor() -> usize {
        <u8 as TreeHash<H>>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> H::Output {
        <u8 as TreeHash<H>>::tree_hash_root(
            &self
                .network_index()
                .expect("btc params should only contain supported networks"),
        )
    }
}

impl AsRef<Params> for BtcParams {
    fn as_ref(&self) -> &Params {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::Network;
    use ssz::{Decode, Encode};
    use tree_hash::{Sha256Hasher, TreeHash};

    use super::*;

    #[test]
    fn test_all_networks_serialization() {
        let networks = [
            Network::Bitcoin,
            Network::Testnet,
            Network::Signet,
            Network::Regtest,
        ];

        for network in networks {
            let params = BtcParams::from(Params::from(network));

            // Test Borsh
            let borsh_data = borsh::to_vec(&params).unwrap();
            let borsh_result = borsh::from_slice::<BtcParams>(&borsh_data).unwrap();
            assert_eq!(params, borsh_result);

            // Test Serde
            let json_data = serde_json::to_string(&params).unwrap();
            let serde_result: BtcParams = serde_json::from_str(&json_data).unwrap();
            assert_eq!(params, serde_result);
        }
    }

    #[test]
    fn test_ssz_roundtrip() {
        let params = BtcParams::from(Params::from(Network::Signet));

        let bytes = params.as_ssz_bytes();
        let decoded = BtcParams::from_ssz_bytes(&bytes).unwrap();

        assert_eq!(params, decoded);
    }

    #[test]
    fn test_tree_hash_deterministic() {
        let params = BtcParams::from(Params::from(Network::Regtest));

        let hash1 = <BtcParams as TreeHash<Sha256Hasher>>::tree_hash_root(&params);
        let hash2 = <BtcParams as TreeHash<Sha256Hasher>>::tree_hash_root(&params);

        assert_eq!(hash1, hash2);
    }
}
