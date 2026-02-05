//! Cryptographic key types and utilities for Strata.
//!
//! This module provides specialized key types used throughout the Strata codebase:
//!
//! - [`constants`] - Derivation paths and constants for key generation
//! - [`compressed`] - Compressed ECDSA public keys with serialization support
//! - [`even`] - Even parity keys for BIP340 Schnorr signatures and taproot
//! - [`zeroizable`] - Zeroizable wrappers for secure key material handling

pub mod compressed;
pub mod constants;
pub mod even;
pub mod zeroizable;

macro_rules! impl_public_key_as_bytes {
    ($name:ident, $array:ty, $to_bytes:expr, $from_bytes:expr) => {
        struct $name;

        impl ArchiveWith<PublicKey> for $name {
            type Archived = Archived<$array>;
            type Resolver = Resolver<$array>;

            fn resolve_with(
                field: &PublicKey,
                resolver: Self::Resolver,
                out: Place<Self::Archived>,
            ) {
                rkyv::Archive::resolve(&($to_bytes)(field), resolver, out);
            }
        }

        impl<S> SerializeWith<PublicKey, S> for $name
        where
            S: Fallible + ?Sized,
            $array: rkyv::Serialize<S>,
        {
            fn serialize_with(
                field: &PublicKey,
                serializer: &mut S,
            ) -> Result<Self::Resolver, S::Error> {
                rkyv::Serialize::serialize(&($to_bytes)(field), serializer)
            }
        }

        impl<D> DeserializeWith<Archived<$array>, PublicKey, D> for $name
        where
            D: Fallible + ?Sized,
            Archived<$array>: rkyv::Deserialize<$array, D>,
        {
            fn deserialize_with(
                field: &Archived<$array>,
                deserializer: &mut D,
            ) -> Result<PublicKey, D::Error> {
                let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
                Ok(($from_bytes)(&bytes))
            }
        }
    };
}

pub(crate) use impl_public_key_as_bytes;
