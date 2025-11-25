//! Compressed ECDSA public key type with Borsh serialization.

use std::ops::Deref;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::ThresholdSigningError;

/// A compressed secp256k1 public key (33 bytes).
///
/// This is a thin wrapper around `secp256k1::PublicKey` that adds Borsh
/// serialization support. Unlike `EvenPublicKey`, this type does not
/// enforce even parity - it accepts any valid compressed public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedPublicKey(PublicKey);

impl CompressedPublicKey {
    /// Create a new `CompressedPublicKey` from a byte slice.
    ///
    /// The slice must be exactly 33 bytes in compressed format (0x02 or 0x03 prefix).
    pub fn from_slice(data: &[u8]) -> Result<Self, ThresholdSigningError> {
        let pk = PublicKey::from_slice(data).map_err(|e| ThresholdSigningError::InvalidPublicKey {
            index: 0,
            reason: e.to_string(),
        })?;
        Ok(Self(pk))
    }

    /// Get the inner `secp256k1::PublicKey`.
    pub fn as_inner(&self) -> &PublicKey {
        &self.0
    }

    /// Serialize to 33-byte compressed format.
    pub fn serialize(&self) -> [u8; 33] {
        self.0.serialize()
    }
}

impl Deref for CompressedPublicKey {
    type Target = PublicKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<PublicKey> for CompressedPublicKey {
    fn as_ref(&self) -> &PublicKey {
        &self.0
    }
}

impl From<PublicKey> for CompressedPublicKey {
    fn from(pk: PublicKey) -> Self {
        Self(pk)
    }
}

impl From<CompressedPublicKey> for PublicKey {
    fn from(pk: CompressedPublicKey) -> Self {
        pk.0
    }
}

impl<'a> Arbitrary<'a> for CompressedPublicKey {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate 32 bytes for a secret key
        let mut sk_bytes = [0u8; 32];
        u.fill_buffer(&mut sk_bytes)?;
        // Ensure we have a valid secret key (non-zero)
        if sk_bytes.iter().all(|&b| b == 0) {
            sk_bytes[31] = 1;
        }
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&sk_bytes)
            .map_err(|_| arbitrary::Error::IncorrectFormat)?;
        let pk = PublicKey::from_secret_key(&secp, &sk);
        Ok(Self(pk))
    }
}

impl BorshSerialize for CompressedPublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let bytes = self.0.serialize();
        writer.write_all(&bytes)
    }
}

impl BorshDeserialize for CompressedPublicKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buf = [0u8; 33];
        reader.read_exact(&mut buf)?;
        let pk = PublicKey::from_slice(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self(pk))
    }
}

#[cfg(feature = "serde")]
impl Serialize for CompressedPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let compressed = self.0.serialize();
        let hex_string = hex::encode(compressed);
        serializer.serialize_str(&hex_string)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for CompressedPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as DeError;

        let hex_string: String = Deserialize::deserialize(deserializer)?;
        let bytes = hex::decode(&hex_string).map_err(DeError::custom)?;
        let pk = PublicKey::from_slice(&bytes).map_err(DeError::custom)?;
        Ok(Self(pk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compressed_pubkey_roundtrip() {
        // Generate a test key
        use secp256k1::{Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);

        let compressed = CompressedPublicKey::from(pk);

        // Test serialization roundtrip
        let bytes = compressed.serialize();
        let restored = CompressedPublicKey::from_slice(&bytes).unwrap();
        assert_eq!(compressed, restored);
    }

    #[test]
    fn test_compressed_pubkey_borsh_roundtrip() {
        use secp256k1::{Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x02; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);

        let compressed = CompressedPublicKey::from(pk);

        // Borsh roundtrip
        let encoded = borsh::to_vec(&compressed).unwrap();
        assert_eq!(encoded.len(), 33);
        let decoded: CompressedPublicKey = borsh::from_slice(&encoded).unwrap();
        assert_eq!(compressed, decoded);
    }

    #[test]
    fn test_invalid_pubkey_slice() {
        let invalid = [0u8; 33];
        let result = CompressedPublicKey::from_slice(&invalid);
        assert!(result.is_err());
    }
}
