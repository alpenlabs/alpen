//! Fixed-size byte buffer types used as building blocks for identifiers.
//!
//! Provides [`Buf20`], [`Buf32`], [`RBuf32`], and [`Buf64`] — thin wrappers
//! around `[u8; N]` arrays with uniform formatting, conversion, and
//! serialization support.
//!
//! [`RBuf32`] is a reversed-display variant of [`Buf32`] that matches the
//! Bitcoin convention of showing hash digests in reversed byte order.
//!
//! # Feature-gated functionality
//!
//! All buffer types conditionally derive additional traits depending on
//! enabled Cargo features:
//!
//! - **`serde`** — JSON and human-readable (de)serialization via hex encoding.
//! - **`ssz`** — SSZ encoding/decoding (available on 32- and 64-byte buffers).
//! - **`borsh`** — Borsh (de)serialization.
//! - **`codec`** — `strata-codec` support.
//! - **`arbitrary`** — `Arbitrary` for fuzz testing.
//! - **`zeroize`** — Secure memory zeroing.

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssz")]
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
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "ssz", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[cfg_attr(feature = "zeroize", derive(zeroize::Zeroize))]
#[repr(transparent)]
pub struct Buf32(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] pub [u8; 32]);
buf_macros::impl_buf_core!(Buf32, 32);
buf_macros::impl_buf_fmt!(Buf32, 32);

#[cfg(feature = "ssz")]
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
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "ssz", derive(Encode, Decode))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[repr(transparent)]
pub struct RBuf32(pub [u8; 32]);
buf_macros::impl_buf_core!(RBuf32, 32);
buf_macros::impl_rbuf_fmt!(RBuf32, 32);
#[cfg(feature = "serde")]
crate::macros::serde_impl::impl_rbuf_serde!(RBuf32, 32);

#[cfg(feature = "ssz")]
crate::impl_ssz_transparent_byte_array_wrapper!(RBuf32, 32);

/// A 64-byte buffer.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "ssz", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[cfg_attr(feature = "zeroize", derive(zeroize::Zeroize))]
pub struct Buf64(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] pub [u8; 64]);
buf_macros::impl_buf_core!(Buf64, 64);
buf_macros::impl_buf_fmt!(Buf64, 64);

#[cfg(feature = "ssz")]
crate::impl_ssz_transparent_byte_array_wrapper!(Buf64, 64);

/// Tests cover behavior from our own macros (`impl_buf_core`, `impl_buf_fmt`,
/// `impl_rbuf_fmt`, `impl_rbuf_serde`, etc.), not derived traits.
#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    // Each type needs its own module because `ssz_proptest!` expands to
    // identically-named test functions (`ssz_roundtrip`, `tree_hash_deterministic`, etc.).
    #[cfg(feature = "ssz")]
    mod ssz {
        use strata_test_utils_ssz::ssz_proptest;

        use super::*;

        mod buf32 {
            use super::*;
            ssz_proptest!(
                Buf32,
                any::<[u8; 32]>(),
                transparent_wrapper_of([u8; 32], from)
            );
        }

        mod rbuf32 {
            use super::*;
            ssz_proptest!(
                RBuf32,
                any::<[u8; 32]>(),
                transparent_wrapper_of([u8; 32], from)
            );
        }

        mod buf64 {
            use super::*;
            ssz_proptest!(
                Buf64,
                any::<[u8; 64]>(),
                transparent_wrapper_of([u8; 64], from)
            );
        }
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

    proptest! {
        #[test]
        fn rbuf32_debug_reverses_byte_order(bytes in any::<[u8; 32]>()) {
            let rbuf = RBuf32::from(bytes);
            let mut reversed = bytes;
            reversed.reverse();
            prop_assert_eq!(format!("{rbuf:?}"), hex::encode(reversed));
        }

        #[test]
        fn rbuf32_display_reverses_byte_order(bytes in any::<[u8; 32]>()) {
            let rbuf = RBuf32::from(bytes);
            let mut reversed = bytes;
            reversed.reverse();
            let expected = format!(
                "{}..{}",
                hex::encode(&reversed[..3]),
                hex::encode(&reversed[29..]),
            );
            prop_assert_eq!(format!("{rbuf}"), expected);
        }
    }

    #[cfg(feature = "serde")]
    proptest! {
        #[test]
        fn rbuf32_json_reverses_byte_order(bytes in any::<[u8; 32]>()) {
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
