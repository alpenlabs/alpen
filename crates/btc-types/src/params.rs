use std::io;

use bitcoin::params::{MAINNET, Params};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode as SszDecodeTrait, DecodeError, Encode as SszEncodeTrait};

#[derive(Debug, Clone)]
pub struct BtcParams(Params);

impl SszEncodeTrait for BtcParams {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        1
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let network_index = match self.0.network {
            bitcoin::Network::Bitcoin => 0u8,
            bitcoin::Network::Testnet => 1u8,
            bitcoin::Network::Signet => 2u8,
            bitcoin::Network::Regtest => 3u8,
            _ => panic!("unsupported bitcoin network"),
        };
        buf.push(network_index);
    }

    fn ssz_bytes_len(&self) -> usize {
        <Self as SszEncodeTrait>::ssz_fixed_len()
    }
}

impl SszDecodeTrait for BtcParams {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        1
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let network_index = u8::from_ssz_bytes(bytes)?;
        let network = match network_index {
            0 => bitcoin::Network::Bitcoin,
            1 => bitcoin::Network::Testnet,
            2 => bitcoin::Network::Signet,
            3 => bitcoin::Network::Regtest,
            _ => {
                return Err(DecodeError::BytesInvalid(format!(
                    "invalid bitcoin network index {network_index}"
                )));
            }
        };
        Ok(Self::from(Params::from(network)))
    }
}

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
        // Serialize the network type as an index since Network doesn't implement BorshSerialize
        let network_index = match self.0.network {
            bitcoin::Network::Bitcoin => 0u8,
            bitcoin::Network::Testnet => 1u8,
            bitcoin::Network::Signet => 2u8,
            bitcoin::Network::Regtest => 3u8,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unsupported network type",
                ));
            }
        };
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

impl AsRef<Params> for BtcParams {
    fn as_ref(&self) -> &Params {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::Network;
    use ssz::{Decode, Encode};

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
    fn test_all_networks_ssz_roundtrip() {
        let networks = [
            Network::Bitcoin,
            Network::Testnet,
            Network::Signet,
            Network::Regtest,
        ];

        for network in networks {
            let params = BtcParams::from(Params::from(network));
            let encoded = params.as_ssz_bytes();
            let decoded = BtcParams::from_ssz_bytes(&encoded).unwrap();

            assert_eq!(params, decoded);
        }
    }
}
