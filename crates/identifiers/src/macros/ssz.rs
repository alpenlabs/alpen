/// Generates SSZ trait implementations for transparent wrappers (generic version).
///
/// This macro generates:
/// - Manual `DecodeView` implementation (using fully qualified path to avoid conflicts)
/// - `SszTypeInfo` implementation for fixed-length types
/// - `TreeHash` implementation that delegates to the inner type
///
/// # Arguments
///
/// * `$wrapper` - The wrapper type
/// * `$inner` - The inner type
/// * `$fixed_len` - The fixed length in bytes (for SszTypeInfo)
///
/// # Example
///
/// ```ignore
/// use ssz_derive::{Decode, Encode};
///
/// type RawAccountId = [u8; 32];
///
/// #[derive(Copy, Clone, Eq, PartialEq, Encode, Decode)]
/// #[ssz(struct_behaviour = "transparent")]
/// pub struct AccountId(RawAccountId);
///
/// impl_ssz_transparent_wrapper!(AccountId, RawAccountId, 32);
/// ```
#[macro_export]
macro_rules! impl_ssz_transparent_wrapper {
    ($wrapper:ty, $inner:ty, $fixed_len:expr) => {
        // Manual DecodeView implementation for transparent wrapper
        // Uses fully qualified path to avoid conflicts with Decode derive
        impl<'a> ::ssz::view::DecodeView<'a> for $wrapper {
            fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ::ssz::DecodeError> {
                Ok(Self(<$inner as ::ssz::view::DecodeView>::from_ssz_bytes(
                    bytes,
                )?))
            }
        }

        // SszTypeInfo implementation for transparent wrapper
        impl ::ssz::view::SszTypeInfo for $wrapper {
            fn is_ssz_fixed_len() -> bool {
                true
            }

            fn ssz_fixed_len() -> usize {
                $fixed_len
            }
        }

        // Manual TreeHash implementation for transparent wrapper
        impl<H: ::tree_hash::TreeHashDigest> ::tree_hash::TreeHash<H> for $wrapper {
            fn tree_hash_type() -> ::tree_hash::TreeHashType {
                <$inner as ::tree_hash::TreeHash<H>>::tree_hash_type()
            }

            fn tree_hash_packed_encoding(&self) -> ::tree_hash::PackedEncoding {
                <$inner as ::tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.0)
            }

            fn tree_hash_packing_factor() -> usize {
                <$inner as ::tree_hash::TreeHash<H>>::tree_hash_packing_factor()
            }

            fn tree_hash_root(&self) -> H::Output {
                <$inner as ::tree_hash::TreeHash<H>>::tree_hash_root(&self.0)
            }
        }
    };
}

/// Generates SSZ trait implementations for transparent wrappers around Buf32.
///
/// This is a convenience macro that calls `impl_ssz_transparent_wrapper!` with Buf32-specific
/// parameters.
///
/// # Example
///
/// ```ignore
/// use crate::Buf32;
/// use ssz_derive::{Decode, Encode};
///
/// #[derive(Copy, Clone, Eq, PartialEq, Encode, Decode)]
/// #[ssz(struct_behaviour = "transparent")]
/// pub struct OLBlockId(pub Buf32);
///
/// impl_ssz_transparent_buf32_wrapper!(OLBlockId);
/// ```
#[macro_export]
macro_rules! impl_ssz_transparent_buf32_wrapper {
    ($wrapper:ty) => {
        $crate::impl_ssz_transparent_wrapper!($wrapper, $crate::buf::Buf32, 32);
    };
}

/// Generates SSZ trait implementations for transparent wrappers around Buf32 that are also Copy.
///
/// This macro generates everything from `impl_ssz_transparent_buf32_wrapper!`.
/// It is kept for clarity at call sites that are specifically `Copy` types.
///
/// # Example
///
/// ```ignore
/// use crate::buf::Buf32;
/// use ssz_derive::{Decode, Encode};
///
/// #[derive(Copy, Clone, Eq, PartialEq, Encode, Decode)]
/// #[ssz(struct_behaviour = "transparent")]
/// pub struct OLTxId(pub Buf32);
///
/// impl_ssz_transparent_buf32_wrapper_copy!(OLTxId);
/// ```
#[macro_export]
macro_rules! impl_ssz_transparent_buf32_wrapper_copy {
    ($wrapper:ty) => {
        $crate::impl_ssz_transparent_buf32_wrapper!($wrapper);
    };
}

/// Generates SSZ trait implementations for transparent wrappers around raw byte arrays `[u8; N]`.
///
/// This macro is specifically for types wrapping raw arrays that don't implement DecodeView.
/// It generates:
/// - Custom `DecodeView` implementation that does array conversion
/// - `SszTypeInfo` implementation for fixed-length types
/// - `TreeHash` implementation that delegates to the inner array
///
/// # Arguments
///
/// * `$wrapper` - The wrapper type
/// * `$len` - The array length (must match the wrapped `[u8; N]`)
///
/// # Example
///
/// ```ignore
/// use ssz_derive::{Decode, Encode};
///
/// type RawAccountId = [u8; 32];
///
/// #[derive(Copy, Clone, Eq, PartialEq, Encode, Decode)]
/// #[ssz(struct_behaviour = "transparent")]
/// pub struct AccountId(RawAccountId);
///
/// impl_ssz_transparent_byte_array_wrapper!(AccountId, 32);
/// ```
#[macro_export]
macro_rules! impl_ssz_transparent_byte_array_wrapper {
    ($wrapper:ty, $len:expr) => {
        // Custom DecodeView implementation for byte array wrapper
        impl<'a> ::ssz::view::DecodeView<'a> for $wrapper {
            fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ::ssz::DecodeError> {
                let array: [u8; $len] =
                    bytes
                        .try_into()
                        .map_err(|_| ::ssz::DecodeError::InvalidByteLength {
                            len: bytes.len(),
                            expected: $len,
                        })?;
                Ok(Self(array))
            }
        }

        // SszTypeInfo implementation for transparent wrapper
        impl ::ssz::view::SszTypeInfo for $wrapper {
            fn is_ssz_fixed_len() -> bool {
                true
            }

            fn ssz_fixed_len() -> usize {
                $len
            }
        }

        // Manual TreeHash implementation for transparent wrapper
        impl<H: ::tree_hash::TreeHashDigest> ::tree_hash::TreeHash<H> for $wrapper {
            fn tree_hash_type() -> ::tree_hash::TreeHashType {
                <[u8; $len] as ::tree_hash::TreeHash<H>>::tree_hash_type()
            }

            fn tree_hash_packed_encoding(&self) -> ::tree_hash::PackedEncoding {
                <[u8; $len] as ::tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.0)
            }

            fn tree_hash_packing_factor() -> usize {
                <[u8; $len] as ::tree_hash::TreeHash<H>>::tree_hash_packing_factor()
            }

            fn tree_hash_root(&self) -> H::Output {
                <[u8; $len] as ::tree_hash::TreeHash<H>>::tree_hash_root(&self.0)
            }
        }
    };
}
