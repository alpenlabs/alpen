//! Even parity key types for Schnorr signatures.
//!
//! This module provides key types that guarantee even parity for the x-only public key,
//! which is required for BIP340 Schnorr signatures and taproot.

use std::ops::Deref;

use borsh::{BorshDeserialize, BorshSerialize};
use hex;
use secp256k1::{Parity, PublicKey, SecretKey, XOnlyPublicKey, SECP256K1};
use serde::{de::Error as DeError, Deserialize, Serialize};
use strata_identifiers::Buf32;

/// A secret key that is guaranteed to have a even x-only public key
#[derive(Debug, Clone, Copy)]
pub struct EvenSecretKey(SecretKey);

impl Deref for EvenSecretKey {
    type Target = SecretKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<SecretKey> for EvenSecretKey {
    fn as_ref(&self) -> &SecretKey {
        &self.0
    }
}

impl From<SecretKey> for EvenSecretKey {
    fn from(value: SecretKey) -> Self {
        match value.x_only_public_key(SECP256K1).1 == Parity::Odd {
            true => Self(value.negate()),
            false => Self(value),
        }
    }
}

impl From<EvenSecretKey> for SecretKey {
    fn from(value: EvenSecretKey) -> Self {
        value.0
    }
}

/// A public key with guaranteed even parity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvenPublicKey(PublicKey);

impl Deref for EvenPublicKey {
    type Target = PublicKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<PublicKey> for EvenPublicKey {
    fn as_ref(&self) -> &PublicKey {
        &self.0
    }
}

impl From<PublicKey> for EvenPublicKey {
    fn from(value: PublicKey) -> Self {
        match value.x_only_public_key().1 == Parity::Odd {
            true => Self(value.negate(SECP256K1)),
            false => Self(value),
        }
    }
}

impl From<EvenPublicKey> for PublicKey {
    fn from(value: EvenPublicKey) -> Self {
        value.0
    }
}

impl From<EvenPublicKey> for XOnlyPublicKey {
    fn from(value: EvenPublicKey) -> Self {
        value.0.x_only_public_key().0
    }
}

impl From<XOnlyPublicKey> for EvenPublicKey {
    fn from(value: XOnlyPublicKey) -> Self {
        // Convert x-only to full public key with even parity
        PublicKey::from_x_only_public_key(value, Parity::Even).into()
    }
}

impl From<EvenPublicKey> for Buf32 {
    fn from(value: EvenPublicKey) -> Self {
        Buf32::from(value.0.x_only_public_key().0.serialize())
    }
}

impl TryFrom<Buf32> for EvenPublicKey {
    type Error = secp256k1::Error;

    fn try_from(value: Buf32) -> Result<Self, Self::Error> {
        let x_only = XOnlyPublicKey::from_slice(value.as_ref())?;
        Ok(PublicKey::from_x_only_public_key(x_only, Parity::Even).into())
    }
}

impl BorshSerialize for EvenPublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let x_only = self.0.x_only_public_key().0;
        BorshSerialize::serialize(&Buf32::from(x_only.serialize()), writer)
    }
}

impl BorshDeserialize for EvenPublicKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let buf = Buf32::deserialize_reader(reader)?;
        let x_only = XOnlyPublicKey::from_slice(buf.as_ref())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(PublicKey::from_x_only_public_key(x_only, Parity::Even).into())
    }
}

impl Serialize for EvenPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as full compressed public key (33 bytes with 0x02 prefix for even parity)
        let compressed = self.0.serialize();
        let hex_string = hex::encode(compressed);
        serializer.serialize_str(&hex_string)
    }
}

impl<'de> Deserialize<'de> for EvenPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex_string: String = Deserialize::deserialize(deserializer)?;
        let bytes = hex::decode(&hex_string).map_err(DeError::custom)?;
        let pk = PublicKey::from_slice(&bytes).map_err(DeError::custom)?;
        // Verify it's even parity
        if pk.x_only_public_key().1 != Parity::Even {
            return Err(DeError::custom(
                "Expected even parity public key, got odd parity",
            ));
        }
        Ok(EvenPublicKey(pk))
    }
}

/// Ensures a keypair is even by checking the public key's parity and negating if odd.
pub fn even_kp((sk, pk): (SecretKey, PublicKey)) -> (EvenSecretKey, EvenPublicKey) {
    match (sk, pk) {
        (sk, pk) if pk.x_only_public_key().1 == Parity::Odd => (
            EvenSecretKey(sk.negate()),
            EvenPublicKey(pk.negate(SECP256K1)),
        ),
        (sk, pk) => (EvenSecretKey(sk), EvenPublicKey(pk)),
    }
}
