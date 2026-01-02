use std::str::FromStr;

#[cfg(feature = "bitcoin")]
use bitcoin::secp256k1::{Error, SecretKey, XOnlyPublicKey, schnorr::Signature};
use const_hex as hex;
use ssz_derive::{Decode, Encode};
use zeroize::Zeroize;

use crate::macros::internal;

/// A 20-byte buffer.
///
/// # Warning
///
/// This type is not zeroized on drop.
/// However, it implements the [`Zeroize`] trait, so you can zeroize it manually.
/// This is useful for secret data that needs to be zeroized after use.
///
/// # Example
///
/// ```
/// # use strata_identifiers::Buf20;
/// use zeroize::Zeroize;
///
/// let mut buf = Buf20::from([1; 20]);
/// buf.zeroize();
///
/// assert_eq!(buf, Buf20::from([0; 20]));
/// ```
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Buf20(pub [u8; 20]);
internal::impl_buf_common!(Buf20, 20);
internal::impl_buf_serde!(Buf20, 20);

// NOTE: we cannot do `ZeroizeOnDrop` since `Buf20` is `Copy`.
impl Zeroize for Buf20 {
    #[inline]
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

/// A 32-byte buffer.
///
/// This is useful for hashes, transaction IDs, secret and public keys.
///
/// # Warning
///
/// This type is not zeroized on drop.
/// However, it implements the [`Zeroize`] trait, so you can zeroize it manually.
/// This is useful for secret data that needs to be zeroized after use.
///
/// # Example
///
/// ```
/// # use strata_identifiers::Buf32;
/// use zeroize::Zeroize;
///
/// let mut buf = Buf32::from([1; 32]);
/// buf.zeroize();
///
/// assert_eq!(buf, Buf32::from([0; 32]));
/// ```
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
pub struct Buf32(pub [u8; 32]);
internal::impl_buf_common!(Buf32, 32);
internal::impl_buf_serde!(Buf32, 32);

crate::impl_ssz_transparent_byte_array_wrapper!(Buf32, 32);

impl FromStr for Buf32 {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode_to_array(s).map(Self::new)
    }
}

#[cfg(feature = "bitcoin")]
impl From<bitcoin::BlockHash> for Buf32 {
    fn from(value: bitcoin::BlockHash) -> Self {
        use bitcoin::hashes::Hash;
        (*value.as_raw_hash().as_byte_array()).into()
    }
}

#[cfg(feature = "bitcoin")]
impl From<bitcoin::Txid> for Buf32 {
    fn from(value: bitcoin::Txid) -> Self {
        use bitcoin::hashes::Hash;
        let bytes: [u8; 32] = *value.as_raw_hash().as_byte_array();
        bytes.into()
    }
}

#[cfg(feature = "bitcoin")]
impl From<&bitcoin::Txid> for Buf32 {
    fn from(value: &bitcoin::Txid) -> Self {
        Self::from(*value)
    }
}

#[cfg(feature = "bitcoin")]
impl From<Buf32> for bitcoin::Txid {
    fn from(value: Buf32) -> Self {
        use bitcoin::hashes::Hash;
        bitcoin::Txid::from_slice(&value.0).expect("valid txid")
    }
}

#[cfg(feature = "bitcoin")]
impl From<bitcoin::Wtxid> for Buf32 {
    fn from(value: bitcoin::Wtxid) -> Self {
        use bitcoin::hashes::Hash;
        let bytes: [u8; 32] = *value.as_raw_hash().as_byte_array();
        bytes.into()
    }
}

#[cfg(feature = "bitcoin")]
impl From<Buf32> for bitcoin::Wtxid {
    fn from(value: Buf32) -> Self {
        use bitcoin::hashes::Hash;
        bitcoin::Wtxid::from_slice(&value.0).expect("valid wtxid")
    }
}

#[cfg(feature = "bitcoin")]
impl From<SecretKey> for Buf32 {
    fn from(value: SecretKey) -> Self {
        let bytes: [u8; 32] = value.secret_bytes();
        bytes.into()
    }
}

#[cfg(feature = "bitcoin")]
impl From<Buf32> for SecretKey {
    fn from(value: Buf32) -> Self {
        SecretKey::from_slice(value.0.as_slice()).expect("could not convert Buf32 into SecretKey")
    }
}

#[cfg(feature = "bitcoin")]
impl TryFrom<Buf32> for XOnlyPublicKey {
    type Error = Error;

    fn try_from(value: Buf32) -> Result<Self, Self::Error> {
        XOnlyPublicKey::from_slice(&value.0)
    }
}

#[cfg(feature = "bitcoin")]
impl From<XOnlyPublicKey> for Buf32 {
    fn from(value: XOnlyPublicKey) -> Self {
        Self::from(value.serialize())
    }
}

#[cfg(feature = "bitcoin")]
impl From<Signature> for Buf64 {
    fn from(value: Signature) -> Self {
        value.serialize().into()
    }
}

// NOTE: we cannot do `ZeroizeOnDrop` since `Buf32` is `Copy`.
impl Zeroize for Buf32 {
    #[inline]
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

/// A 64-byte buffer.
///
/// This is useful for schnorr signatures.
///
/// # Warning
///
/// This type is not zeroized on drop.
/// However, it implements the [`Zeroize`] trait, so you can zeroize it manually.
/// This is useful for secret data that needs to be zeroized after use.
///
/// # Example
///
/// ```
/// # use strata_identifiers::Buf64;
/// use zeroize::Zeroize;
///
/// let mut buf = Buf64::from([1; 64]);
/// buf.zeroize();
///
/// assert_eq!(buf, Buf64::from([0; 64]));
/// ```
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
pub struct Buf64(pub [u8; 64]);
internal::impl_buf_common!(Buf64, 64);
internal::impl_buf_serde!(Buf64, 64);

crate::impl_ssz_transparent_byte_array_wrapper!(Buf64, 64);

// NOTE: we cannot do `ZeroizeOnDrop` since `Buf64` is `Copy`.
impl Zeroize for Buf64 {
    #[inline]
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod buf32_ssz {
        use super::*;

        ssz_proptest!(
            Buf32,
            any::<[u8; 32]>(),
            transparent_wrapper_of([u8; 32], from)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = Buf32::zero();
            let encoded = zero.as_ssz_bytes();
            let decoded = Buf32::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    mod buf64_ssz {
        use super::*;

        ssz_proptest!(
            Buf64,
            any::<[u8; 64]>(),
            transparent_wrapper_of([u8; 64], from)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = Buf64::from([0u8; 64]);
            let encoded = zero.as_ssz_bytes();
            let decoded = Buf64::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    #[test]
    fn test_buf32_deserialization() {
        // without 0x
        assert_eq!(
            Buf32::from([0; 32]),
            serde_json::from_str(
                "\"0000000000000000000000000000000000000000000000000000000000000000\"",
            )
            .unwrap()
        );

        // with 0x
        assert_eq!(
            Buf32::from([1; 32]),
            serde_json::from_str(
                "\"0x0101010101010101010101010101010101010101010101010101010101010101\"",
            )
            .unwrap()
        );

        // correct byte order
        assert_eq!(
            Buf32::from([
                1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
                1, 1, 1, 170u8
            ]),
            serde_json::from_str(
                "\"0x01010101010101010101010101010101010101010101010101010101010101aa\"",
            )
            .unwrap()
        );
    }

    #[test]
    fn test_buf32_serialization() {
        assert_eq!(
            serde_json::to_string(&Buf32::from([0; 32])).unwrap(),
            String::from("\"0000000000000000000000000000000000000000000000000000000000000000\"")
        );

        assert_eq!(
            serde_json::to_string(&Buf32::from([
                1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
                1, 1, 1, 170u8
            ]))
            .unwrap(),
            String::from("\"01010101010101010101010101010101010101010101010101010101010101aa\"")
        );
    }

    #[test]
    fn test_zeroize() {
        let mut buf20 = Buf20::from([1; 20]);
        let mut buf32 = Buf32::from([1; 32]);
        let mut buf64 = Buf64::from([1; 64]);
        buf20.zeroize();
        buf32.zeroize();
        buf64.zeroize();
        assert_eq!(buf20, Buf20::from([0; 20]));
        assert_eq!(buf32, Buf32::from([0; 32]));
        assert_eq!(buf64, Buf64::from([0; 64]));
    }

    #[test]
    fn test_buf32_parse() {
        "0x37ad61cff1367467a98cf7c54c4ac99e989f1fbb1bc1e646235e90c065c565ba"
            .parse::<Buf32>()
            .unwrap();
    }

    #[test]
    fn test_buf32_from_str() {
        Buf32::from_str("a9f913c3d7fe56c462228ad22bb7631742a121a6a138d57c1fc4a351314948fa")
            .unwrap();

        Buf32::from_str("81060cb3997dcefc463e3db0a776efb5360e458064a666459b8807f60c0201c2")
            .unwrap();
    }
}
