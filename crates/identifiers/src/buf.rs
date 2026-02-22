use std::str::FromStr;

use const_hex as hex;
use ssz_derive::{Decode, Encode};
use ssz_primitives::FixedBytes;
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
internal::impl_buf_core!(Buf20, 20);
internal::impl_buf_fmt!(Buf20, 20);
internal::impl_buf_borsh!(Buf20, 20);
internal::impl_buf_arbitrary!(Buf20, 20);
internal::impl_buf_codec!(Buf20, 20);
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
internal::impl_buf_core!(Buf32, 32);
internal::impl_buf_fmt!(Buf32, 32);
internal::impl_buf_borsh!(Buf32, 32);
internal::impl_buf_arbitrary!(Buf32, 32);
internal::impl_buf_codec!(Buf32, 32);
internal::impl_buf_serde!(Buf32, 32);

crate::impl_ssz_transparent_byte_array_wrapper!(Buf32, 32);

impl FromStr for Buf32 {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode_to_array(s).map(Self::new)
    }
}

/// A 32-byte buffer with reversed-byte display and serialization.
///
/// Stores bytes internally in their natural (little-endian) order but
/// reverses them for [`Display`], [`Debug`], and human-readable serde.
/// This matches the Bitcoin convention where block hashes, transaction
/// IDs, and other hash digests are displayed in reversed byte order.
///
/// Use this instead of [`Buf32`] when the value represents a Bitcoin
/// type (e.g., `BlockHash`, `Txid`, `Wtxid`) that follows this
/// convention.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
pub struct RBuf32(pub [u8; 32]);
internal::impl_buf_core!(RBuf32, 32);
internal::impl_rbuf_fmt!(RBuf32, 32);
internal::impl_buf_borsh!(RBuf32, 32);
internal::impl_buf_arbitrary!(RBuf32, 32);
internal::impl_buf_codec!(RBuf32, 32);
internal::impl_rbuf_serde!(RBuf32, 32);

crate::impl_ssz_transparent_byte_array_wrapper!(RBuf32, 32);

impl From<FixedBytes<32>> for Buf32 {
    fn from(value: FixedBytes<32>) -> Self {
        Buf32(value.0)
    }
}

impl From<&FixedBytes<32>> for &Buf32 {
    fn from(value: &FixedBytes<32>) -> Self {
        // SAFETY: FixedBytes<32> and Buf32 have the same layout
        unsafe { &*(value as *const FixedBytes<32> as *const Buf32) }
    }
}

impl From<Buf32> for FixedBytes<32> {
    fn from(value: Buf32) -> Self {
        FixedBytes(value.0)
    }
}

impl From<&Buf32> for &FixedBytes<32> {
    fn from(value: &Buf32) -> Self {
        // SAFETY: Buf32 and FixedBytes<32> have the same layout
        unsafe { &*(value as *const Buf32 as *const FixedBytes<32>) }
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
internal::impl_buf_core!(Buf64, 64);
internal::impl_buf_fmt!(Buf64, 64);
internal::impl_buf_borsh!(Buf64, 64);
internal::impl_buf_arbitrary!(Buf64, 64);
internal::impl_buf_codec!(Buf64, 64);
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
