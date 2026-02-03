//! Even parity key types for Schnorr signatures.
//!
//! This module provides key types that guarantee even parity for the x-only public key,
//! which is required for BIP340 Schnorr signatures and taproot.

use std::ops::Deref;

use hex;
use rkyv::{
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
    Archived, Place, Resolver,
};
use secp256k1::{Parity, PublicKey, SecretKey, XOnlyPublicKey, SECP256K1};
use serde::{de::Error as DeError, Deserialize, Serialize};
use strata_identifiers::Buf32;

/// Represents a secret key whose x-only public key has even parity.
///
/// Converting from a [`SecretKey`] negates the key when its x-only public key has odd parity,
/// so the resulting [`EvenSecretKey`] always yields even parity.
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

/// Represents a public key whose x-only public key has even parity.
///
/// Converting from a [`PublicKey`] negates the key when its x-only public key has odd parity,
/// so the resulting [`EvenPublicKey`] always yields even parity.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct EvenPublicKey(#[rkyv(with = EvenPublicKeyAsBytes)] PublicKey);

/// Serializer for [`PublicKey`] as bytes for rkyv.
struct EvenPublicKeyAsBytes;

impl ArchiveWith<PublicKey> for EvenPublicKeyAsBytes {
    type Archived = Archived<[u8; 32]>;
    type Resolver = Resolver<[u8; 32]>;

    fn resolve_with(field: &PublicKey, resolver: Self::Resolver, out: Place<Self::Archived>) {
        let x_only = field.x_only_public_key().0;
        rkyv::Archive::resolve(&x_only.serialize(), resolver, out);
    }
}

impl<S> SerializeWith<PublicKey, S> for EvenPublicKeyAsBytes
where
    S: Fallible + ?Sized,
    [u8; 32]: rkyv::Serialize<S>,
{
    fn serialize_with(field: &PublicKey, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        let x_only = field.x_only_public_key().0;
        rkyv::Serialize::serialize(&x_only.serialize(), serializer)
    }
}

impl<D> DeserializeWith<Archived<[u8; 32]>, PublicKey, D> for EvenPublicKeyAsBytes
where
    D: Fallible + ?Sized,
    Archived<[u8; 32]>: rkyv::Deserialize<[u8; 32], D>,
{
    fn deserialize_with(
        field: &Archived<[u8; 32]>,
        deserializer: &mut D,
    ) -> Result<PublicKey, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        let x_only = XOnlyPublicKey::from_slice(&bytes).expect("stored public key should decode");
        Ok(PublicKey::from_x_only_public_key(x_only, Parity::Even))
    }
}

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

#[cfg(test)]
mod tests {
    use rkyv::rancor::Error as RkyvError;
    use secp256k1::{Parity, PublicKey, SecretKey, SECP256K1};
    use strata_identifiers::Buf32;

    use super::{even_kp, EvenPublicKey, EvenSecretKey};

    fn sample_secret_keys() -> (SecretKey, SecretKey) {
        let sk = SecretKey::from_slice(&[0x01; 32]).expect("valid secret key");
        let sk_neg = sk.negate();
        match sk.x_only_public_key(SECP256K1).1 {
            Parity::Even => (sk, sk_neg),
            Parity::Odd => (sk_neg, sk),
        }
    }

    fn sample_public_keys() -> (PublicKey, PublicKey) {
        let (even_sk, odd_sk) = sample_secret_keys();
        let even_pk = PublicKey::from_secret_key(SECP256K1, &even_sk);
        let odd_pk = PublicKey::from_secret_key(SECP256K1, &odd_sk);
        (even_pk, odd_pk)
    }

    #[test]
    fn test_even_secret_key_from_parity() {
        let (even_sk, odd_sk) = sample_secret_keys();

        let from_even = EvenSecretKey::from(even_sk);
        assert_eq!(from_even.x_only_public_key(SECP256K1).1, Parity::Even);
        assert_eq!(SecretKey::from(from_even), even_sk);

        let from_odd = EvenSecretKey::from(odd_sk);
        assert_eq!(from_odd.x_only_public_key(SECP256K1).1, Parity::Even);
        assert_eq!(SecretKey::from(from_odd), odd_sk.negate());
    }

    #[test]
    fn test_even_public_key_from_parity() {
        let (even_pk, odd_pk) = sample_public_keys();

        let from_even = EvenPublicKey::from(even_pk);
        assert_eq!(from_even.x_only_public_key().1, Parity::Even);
        assert_eq!(PublicKey::from(from_even), even_pk);

        let from_odd = EvenPublicKey::from(odd_pk);
        assert_eq!(from_odd.x_only_public_key().1, Parity::Even);
        assert_eq!(PublicKey::from(from_odd), odd_pk.negate(SECP256K1));
    }

    #[test]
    fn test_even_public_key_rkyv_roundtrip() {
        let (even_pk, _) = sample_public_keys();
        let even_pk = EvenPublicKey::from(even_pk);

        let encoded = rkyv::to_bytes::<RkyvError>(&even_pk).expect("rkyv encode");
        let decoded: EvenPublicKey =
            rkyv::from_bytes::<EvenPublicKey, RkyvError>(&encoded).expect("rkyv decode");

        assert_eq!(even_pk, decoded);
    }

    #[test]
    fn test_even_public_key_buf32_roundtrip() {
        let (even_pk, _) = sample_public_keys();
        let even_pk = EvenPublicKey::from(even_pk);

        let buf = Buf32::from(even_pk);
        let decoded = EvenPublicKey::try_from(buf).expect("valid x-only key");

        assert_eq!(even_pk, decoded);
    }

    #[test]
    fn test_even_kp_negates_on_odd_parity() {
        let (_, odd_sk) = sample_secret_keys();
        let odd_pk = PublicKey::from_secret_key(SECP256K1, &odd_sk);

        let (even_sk, even_pk) = even_kp((odd_sk, odd_pk));

        assert_eq!(SecretKey::from(even_sk), odd_sk.negate());
        assert_eq!(PublicKey::from(even_pk), odd_pk.negate(SECP256K1));
    }
}
