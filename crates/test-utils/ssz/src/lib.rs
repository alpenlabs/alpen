//! Test utilities for SSZ types.
//!
//! This crate provides macros for property-based testing of SSZ-serializable types.

#![allow(
    unused_crate_dependencies,
    reason = "dependencies are used in macro expansions"
)]

// Re-export dependencies for use in macro expansions
#[doc(hidden)]
pub use proptest;
#[doc(hidden)]
pub use ssz;
#[doc(hidden)]
pub use tree_hash::{self, Sha256Hasher};

/// Generates property-based tests for SSZ encoding/decoding and tree hashing.
///
/// # Basic variant
/// Creates two proptest tests:
/// 1. SSZ roundtrip test - verifies encode/decode works correctly
/// 2. Tree hash determinism test - verifies tree hash is deterministic
///
/// # Transparent wrapper variant
/// Creates three proptest tests:
/// 1. SSZ roundtrip test - verifies encode/decode works correctly
/// 2. Tree hash determinism test - verifies tree hash is deterministic
/// 3. Tree hash transparency test - verifies hash matches inner type
///
/// # Examples
///
/// Basic usage:
/// ```ignore
/// use strata_test_utils_ssz::ssz_proptest;
/// use proptest::prelude::*;
///
/// ssz_proptest!(MyType, any::<u64>().prop_map(MyType::new));
/// ```
///
/// For transparent wrappers:
/// ```ignore
/// ssz_proptest!(
///     BitcoinAmount,
///     any::<u64>(),
///     transparent_wrapper_of(u64, from_sat)
/// );
/// ```
/// Wrapper around `proptest::proptest!` that skips tests under miri.
///
/// Miri runs significantly slower than normal execution, making proptest's
/// many randomized iterations impractical (tests can take hours). This macro
/// conditionally compiles out proptest tests when running under miri.
#[macro_export]
#[cfg(not(miri))]
macro_rules! ssz_proptest {
    // Variant without transparent wrapper
    ($type:ty, $strategy:expr) => {
        $crate::proptest::proptest! {
            #[test]
            fn ssz_roundtrip(val in $strategy) {
                use $crate::ssz::{Encode, Decode};
                let encoded = val.as_ssz_bytes();
                let decoded = <$type>::from_ssz_bytes(&encoded).unwrap();
                $crate::proptest::prop_assert_eq!(val, decoded);
            }

            #[test]
            fn tree_hash_deterministic(val in $strategy) {
                use $crate::tree_hash::{TreeHash, Sha256Hasher};
                let hash1 = <$type as TreeHash<Sha256Hasher>>::tree_hash_root(&val);
                let hash2 = <$type as TreeHash<Sha256Hasher>>::tree_hash_root(&val);
                $crate::proptest::prop_assert_eq!(hash1, hash2);
            }
        }
    };

    // Variant with transparent wrapper - tests that tree hash matches inner type
    ($type:ty, $inner_strategy:expr, transparent_wrapper_of($inner:ty, $constructor:ident)) => {
        $crate::proptest::proptest! {
            #[test]
            fn ssz_roundtrip(inner_val in $inner_strategy) {
                use $crate::ssz::{Encode, Decode};
                let val = <$type>::$constructor(inner_val);
                let encoded = val.as_ssz_bytes();
                let decoded = <$type>::from_ssz_bytes(&encoded).unwrap();
                $crate::proptest::prop_assert_eq!(val, decoded);
            }

            #[test]
            fn tree_hash_deterministic(inner_val in $inner_strategy) {
                use $crate::tree_hash::{TreeHash, Sha256Hasher};
                let val = <$type>::$constructor(inner_val);
                let hash1 = <$type as TreeHash<Sha256Hasher>>::tree_hash_root(&val);
                let hash2 = <$type as TreeHash<Sha256Hasher>>::tree_hash_root(&val);
                $crate::proptest::prop_assert_eq!(hash1, hash2);
            }

            #[test]
            fn tree_hash_transparent(inner_val in $inner_strategy) {
                use $crate::tree_hash::{TreeHash, Sha256Hasher};
                let val = <$type>::$constructor(inner_val);
                let wrapper_hash = <$type as TreeHash<Sha256Hasher>>::tree_hash_root(&val);
                let inner_hash = <$inner as TreeHash<Sha256Hasher>>::tree_hash_root(&inner_val);
                $crate::proptest::prop_assert_eq!(wrapper_hash, inner_hash);
            }
        }
    };
}

/// No-op version of [`ssz_proptest!`] when running under miri.
///
/// Proptest tests are too slow under miri's interpreted execution, so we skip them entirely.
#[macro_export]
#[cfg(miri)]
macro_rules! ssz_proptest {
    ($($tt:tt)*) => {};
}
