#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
#[cfg(feature = "codec")]
use strata_codec::Codec;

use crate::macros::buf as buf_macros;

/// A 20-byte buffer.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[cfg_attr(feature = "zeroize", derive(zeroize::Zeroize))]
pub struct Buf20(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] pub [u8; 20]);
buf_macros::impl_buf_core!(Buf20, 20);
buf_macros::impl_buf_fmt!(Buf20, 20);

/// A 32-byte buffer.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[cfg_attr(feature = "zeroize", derive(zeroize::Zeroize))]
#[repr(transparent)]
pub struct Buf32(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] pub [u8; 32]);
buf_macros::impl_buf_core!(Buf32, 32);
buf_macros::impl_buf_fmt!(Buf32, 32);

crate::impl_ssz_transparent_byte_array_wrapper!(Buf32, 32);

/// A 32-byte buffer with reversed-byte display and serialization.
///
/// Stores bytes internally in their natural (little-endian) order but
/// reverses them for [`Display`](std::fmt::Display), [`Debug`], and human-readable serde.
/// This matches the Bitcoin convention where block hashes, transaction
/// IDs, and other hash digests are displayed in reversed byte order.
///
/// Use this instead of [`Buf32`] when the value represents a Bitcoin
/// type (e.g., `BlockHash`, `Txid`, `Wtxid`) that follows this
/// convention.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[repr(transparent)]
pub struct RBuf32(pub [u8; 32]);
buf_macros::impl_buf_core!(RBuf32, 32);
buf_macros::impl_rbuf_fmt!(RBuf32, 32);
#[cfg(feature = "serde")]
crate::macros::serde_impl::impl_rbuf_serde!(RBuf32, 32);

crate::impl_ssz_transparent_byte_array_wrapper!(RBuf32, 32);

/// A 64-byte buffer.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[cfg_attr(feature = "zeroize", derive(zeroize::Zeroize))]
pub struct Buf64(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] pub [u8; 64]);
buf_macros::impl_buf_core!(Buf64, 64);
buf_macros::impl_buf_fmt!(Buf64, 64);

crate::impl_ssz_transparent_byte_array_wrapper!(Buf64, 64);

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use proptest::prelude::*;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod buf32_ssz {
        use super::*;

        ssz_proptest!(
            Buf32,
            any::<[u8; 32]>(),
            transparent_wrapper_of([u8; 32], from)
        );
    }

    mod buf64_ssz {
        use super::*;

        ssz_proptest!(
            Buf64,
            any::<[u8; 64]>(),
            transparent_wrapper_of([u8; 64], from)
        );
    }

    #[test]
    fn test_buf32_deserialization() {
        assert_eq!(
            Buf32::from([0; 32]),
            serde_json::from_str(
                "\"0000000000000000000000000000000000000000000000000000000000000000\"",
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
                "\"01010101010101010101010101010101010101010101010101010101010101aa\"",
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

    #[cfg(feature = "zeroize")]
    #[test]
    fn test_zeroize() {
        use zeroize::Zeroize;

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

    mod rbuf32_serde {
        use super::*;

        proptest! {
            #[test]
            fn json_reverses_byte_order(bytes in any::<[u8; 32]>()) {
                let buf = Buf32::from(bytes);
                let rbuf = RBuf32::from(bytes);
                let buf_json: String = serde_json::from_str(&serde_json::to_string(&buf).unwrap()).unwrap();
                let rbuf_json: String = serde_json::from_str(&serde_json::to_string(&rbuf).unwrap()).unwrap();
                let mut reversed_bytes = bytes;
                reversed_bytes.reverse();
                prop_assert_eq!(&rbuf_json, &hex::encode(reversed_bytes));
                prop_assert_eq!(&buf_json, &hex::encode(bytes));
            }
        }
    }

    #[test]
    fn test_buf32_from_str() {
        Buf32::from_str("a9f913c3d7fe56c462228ad22bb7631742a121a6a138d57c1fc4a351314948fa")
            .unwrap();

        Buf32::from_str("81060cb3997dcefc463e3db0a776efb5360e458064a666459b8807f60c0201c2")
            .unwrap();
    }
}
