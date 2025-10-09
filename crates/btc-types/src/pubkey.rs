use std::io::{Read, Write};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{key::TapTweak, secp256k1::XOnlyPublicKey, Address, AddressType, Network, ScriptBuf};
use bitcoin_bosd::Descriptor;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

use crate::{BitcoinAddress, ParseError};

/// A wrapper around [`Buf32`] for XOnly Schnorr taproot pubkeys.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct XOnlyPk(Buf32);

impl XOnlyPk {
    /// Construct a new [`XOnlyPk`] directly from a [`Buf32`].
    pub fn new(val: Buf32) -> Result<Self, ParseError> {
        if Self::is_valid_xonly_public_key(&val) {
            Ok(Self(val))
        } else {
            Err(ParseError::InvalidPoint(val))
        }
    }

    /// Get the underlying [`Buf32`].
    pub fn inner(&self) -> &Buf32 {
        &self.0
    }

    /// Convert a [`BitcoinAddress`] into a [`XOnlyPk`].
    pub fn from_address(checked_addr: &BitcoinAddress) -> Result<Self, ParseError> {
        let checked_addr = checked_addr.address();

        if let Some(AddressType::P2tr) = checked_addr.address_type() {
            let script_pubkey = checked_addr.script_pubkey();

            // skip the version and length bytes
            let pubkey_bytes = &script_pubkey.as_bytes()[2..34];
            let output_key: XOnlyPublicKey = XOnlyPublicKey::from_slice(pubkey_bytes)?;

            Ok(Self(Buf32(output_key.serialize())))
        } else {
            Err(ParseError::UnsupportedAddress(checked_addr.address_type()))
        }
    }

    /// Convert the [`XOnlyPk`] to a `rust-bitcoin`'s [`XOnlyPublicKey`].
    pub fn to_xonly_public_key(&self) -> XOnlyPublicKey {
        XOnlyPublicKey::from_slice(self.0.as_bytes()).expect("XOnlyPk is valid")
    }

    /// Convert the [`XOnlyPk`] to an [`Address`].
    pub fn to_p2tr_address(&self, network: Network) -> Result<Address, ParseError> {
        let buf: [u8; 32] = self.0.0;
        let pubkey = XOnlyPublicKey::from_slice(&buf)?;

        Ok(Address::p2tr_tweaked(
            pubkey.dangerous_assume_tweaked(),
            network,
        ))
    }

    /// Converts [`XOnlyPk`] to [`Descriptor`].
    pub fn to_descriptor(&self) -> Result<Descriptor, ParseError> {
        Descriptor::new_p2tr(&self.to_xonly_public_key().serialize())
            .map_err(|_| ParseError::InvalidPoint(self.0))
    }

    /// Checks if the [`Buf32`] is a valid [`XOnlyPublicKey`].
    fn is_valid_xonly_public_key(buf: &Buf32) -> bool {
        XOnlyPublicKey::from_slice(buf.as_bytes()).is_ok()
    }
}

impl From<XOnlyPublicKey> for XOnlyPk {
    fn from(value: XOnlyPublicKey) -> Self {
        Self(Buf32(value.serialize()))
    }
}

impl TryFrom<XOnlyPk> for Descriptor {
    type Error = ParseError;

    fn try_from(value: XOnlyPk) -> Result<Self, Self::Error> {
        let inner_xonly_pk = XOnlyPublicKey::try_from(value.0)?;
        Ok(inner_xonly_pk.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BitcoinScriptBuf(ScriptBuf);

impl BitcoinScriptBuf {
    pub fn inner(&self) -> &ScriptBuf {
        &self.0
    }
}

impl From<ScriptBuf> for BitcoinScriptBuf {
    fn from(value: ScriptBuf) -> Self {
        Self(value)
    }
}

// Implement BorshSerialize for BitcoinScriptBuf
impl BorshSerialize for BitcoinScriptBuf {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let script_bytes = self.0.to_bytes();
        BorshSerialize::serialize(&(script_bytes.len() as u32), writer)?;
        writer.write_all(&script_bytes)?;
        Ok(())
    }
}

// Implement BorshDeserialize for BitcoinScriptBuf
impl BorshDeserialize for BitcoinScriptBuf {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let script_len = u32::deserialize_reader(reader)? as usize;
        let mut script_bytes = vec![0u8; script_len];
        reader.read_exact(&mut script_bytes)?;
        let script_pubkey = ScriptBuf::from(script_bytes);

        Ok(BitcoinScriptBuf(script_pubkey))
    }
}

impl<'a> Arbitrary<'a> for BitcoinScriptBuf {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate arbitrary script
        let script_len = usize::arbitrary(u)? % 100; // Limit script length
        let script_bytes = u.bytes(script_len)?;
        let script = ScriptBuf::from(script_bytes.to_vec());

        Ok(Self(script))
    }
}
