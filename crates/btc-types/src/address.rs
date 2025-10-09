use std::io::{self, Read, Write};

use bitcoin::{address::NetworkUnchecked, Address, Network, ScriptBuf};
use bitcoin_bosd::Descriptor;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{de, Deserialize, Deserializer, Serialize};

use crate::ParseError;

/// A wrapper around the [`bitcoin::Address<NetworkChecked>`] type.
///
/// It's created in order to implement some useful traits on it such as
/// [`serde::Deserialize`], [`borsh::BorshSerialize`] and [`borsh::BorshDeserialize`].
// TODO: implement [`arbitrary::Arbitrary`]?
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct BitcoinAddress {
    /// The [`bitcoin::Network`] that this address is valid in.
    network: Network,

    /// The actual [`Address`] that this type wraps.
    address: Address,
}

impl BitcoinAddress {
    /// Parses a [`BitcoinAddress`] from a string.
    pub fn parse(address_str: &str, network: Network) -> Result<Self, ParseError> {
        let address = address_str
            .parse::<Address<NetworkUnchecked>>()
            .map_err(ParseError::InvalidAddress)?;

        let checked_address = address
            .require_network(network)
            .map_err(ParseError::InvalidAddress)?;

        Ok(Self {
            network,
            address: checked_address,
        })
    }

    /// Parses a [`BitcoinAddress`] from raw bytes representation of a bitcoin Script.
    pub fn from_bytes(bytes: &[u8], network: Network) -> Result<Self, ParseError> {
        let script_buf = ScriptBuf::from_bytes(bytes.to_vec());
        let address = Address::from_script(&script_buf, network)?;
        Ok(Self { network, address })
    }

    pub fn from_descriptor(descriptor: &Descriptor, network: Network) -> Result<Self, ParseError> {
        let address = descriptor
            .to_address(network)
            .map_err(|err| ParseError::Descriptor(err.to_string()))?;
        Ok(Self { network, address })
    }
}

impl BitcoinAddress {
    pub fn address(&self) -> &Address {
        &self.address
    }

    pub fn network(&self) -> &Network {
        &self.network
    }
}

impl<'de> Deserialize<'de> for BitcoinAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct BitcoinAddressShim {
            network: Network,
            address: String,
        }

        let shim = BitcoinAddressShim::deserialize(deserializer)?;
        let address = shim
            .address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| de::Error::custom("invalid bitcoin address"))?
            .require_network(shim.network)
            .map_err(|_| de::Error::custom("address invalid for given network"))?;

        Ok(BitcoinAddress {
            network: shim.network,
            address,
        })
    }
}

impl BorshSerialize for BitcoinAddress {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        let address_string = self.address.to_string();

        BorshSerialize::serialize(address_string.as_str(), writer)?;

        let network_byte = match self.network {
            Network::Bitcoin => 0u8,
            Network::Testnet => 1u8,
            Network::Signet => 2u8,
            Network::Regtest => 3u8,
            other => unreachable!("should handle new variant: {}", other),
        };

        BorshSerialize::serialize(&network_byte, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for BitcoinAddress {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, io::Error> {
        let address_str = String::deserialize_reader(reader)?;

        let network_byte = u8::deserialize_reader(reader)?;
        let network = match network_byte {
            0u8 => Network::Bitcoin,
            1u8 => Network::Testnet,
            2u8 => Network::Signet,
            3u8 => Network::Regtest,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid network byte: {network_byte}"),
                ));
            }
        };

        let address = address_str
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid bitcoin address"))?
            .require_network(network)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "address invalid for given network",
                )
            })?;

        Ok(BitcoinAddress { address, network })
    }
}
