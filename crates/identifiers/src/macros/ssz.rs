/// Generates a generic `TreeHash<H>` implementation for an SSZ container type.
///
/// The `tree_hash_derive::TreeHash` derive macro generates a concrete
/// `impl TreeHash<Sha256Hasher>`, but `ssz_codegen`-generated code in downstream
/// crates requires `impl<H: TreeHashDigest> TreeHash<H>`. This macro produces
/// the generic version.
///
/// # Arguments
///
/// * `$type` - The container type (must have named fields)
/// * `[$($field:ident),+]` - List of field names to include in the tree hash
///
/// # Example
///
/// ```ignore
/// #[derive(Encode, Decode)]
/// #[ssz(struct_behaviour = "container")]
/// pub struct MyContainer {
///     pub a: u64,
///     pub b: Buf32,
/// }
///
/// impl_tree_hash_container!(MyContainer, [a, b]);
/// ```
#[macro_export]
macro_rules! impl_tree_hash_container {
    ($type:ty, [$($field:ident),+ $(,)?]) => {
        impl<H: ::tree_hash::TreeHashDigest> ::tree_hash::TreeHash<H> for $type {
            fn tree_hash_type() -> ::tree_hash::TreeHashType {
                ::tree_hash::TreeHashType::Container
            }

            fn tree_hash_packed_encoding(&self) -> ::tree_hash::PackedEncoding {
                unreachable!("Container should never be packed")
            }

            fn tree_hash_packing_factor() -> usize {
                unreachable!("Container should never be packed")
            }

            fn tree_hash_root(&self) -> H::Output {
                let mut hasher = ::tree_hash::MerkleHasher::<H>::with_leaves(
                    $crate::impl_tree_hash_container!(@count $($field),+)
                );
                $(
                    hasher
                        .write(
                            <_ as ::tree_hash::TreeHash<H>>::tree_hash_root(&self.$field)
                                .as_ref(),
                        )
                        .expect("tree hash derive should not apply too many leaves");
                )+
                hasher
                    .finish()
                    .expect("tree hash derive should not have a remaining buffer")
            }
        }
    };
    // Internal helper: count the number of fields
    (@count $head:ident $(, $tail:ident)*) => {
        1usize $(+ $crate::impl_tree_hash_container!(@count_one $tail))*
    };
    (@count_one $x:ident) => { 1usize };
}

/// Implements `SszTypeInfo` for a fixed-size SSZ container type.
///
/// All fields must be fixed-size SSZ types. The total size is the sum of
/// all field sizes.
///
/// # Arguments
///
/// * `$type` - The container type
/// * `[$($field_ty:ty),+]` - List of field types (in order) to compute the fixed size
#[macro_export]
macro_rules! impl_ssz_type_info_fixed {
    ($type:ty, [$($field_ty:ty),+ $(,)?]) => {
        impl ::ssz::view::SszTypeInfo for $type {
            fn is_ssz_fixed_len() -> bool {
                true
            }

            fn ssz_fixed_len() -> usize {
                0 $(+ <$field_ty as ::ssz::view::SszTypeInfo>::ssz_fixed_len())+
            }
        }
    };
}

/// Generates a zero-copy `Ref` view type for an SSZ container.
///
/// Downstream crates using `ssz_codegen` reference `TypeRef<'a>` types for
/// imported containers. This macro generates a lightweight `Ref` type that
/// eagerly decodes to the owned type (appropriate for small, fixed-size
/// containers).
///
/// Generates:
/// - A `Ref` struct wrapping the owned type with a phantom lifetime
/// - `DecodeView<'a>` (decodes via `ssz::Decode`)
/// - `SszTypeInfo` (delegates to owned type)
/// - `TreeHash<H>` (delegates to owned type)
/// - `ToOwnedSsz` (returns inner owned value)
///
/// # Arguments
///
/// * `$ref_name` - Name for the ref type (e.g., `OLBlockCommitmentRef`)
/// * `$owned` - The owned container type
///
/// # Example
///
/// ```ignore
/// impl_ssz_container_ref!(OLBlockCommitmentRef, OLBlockCommitment);
/// ```
#[macro_export]
macro_rules! impl_ssz_container_ref {
    ($ref_name:ident, $owned:ty) => {
        #[derive(
            Copy,
            Clone,
            Debug,
            PartialEq,
            Eq,
            Hash,
            Default,
            serde::Serialize,
            serde::Deserialize,
        )]
        pub struct $ref_name<'a> {
            inner: $owned,
            _phantom: ::std::marker::PhantomData<&'a ()>,
        }

        impl<'a> ::ssz::view::DecodeView<'a> for $ref_name<'a> {
            fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ::ssz::DecodeError> {
                let inner = <$owned as ::ssz::Decode>::from_ssz_bytes(bytes)?;
                Ok(Self {
                    inner,
                    _phantom: ::std::marker::PhantomData,
                })
            }
        }

        impl<'a> ::ssz::view::SszTypeInfo for $ref_name<'a> {
            fn is_ssz_fixed_len() -> bool {
                <$owned as ::ssz::view::SszTypeInfo>::is_ssz_fixed_len()
            }

            fn ssz_fixed_len() -> usize {
                <$owned as ::ssz::view::SszTypeInfo>::ssz_fixed_len()
            }
        }

        impl<'a, H: ::tree_hash::TreeHashDigest> ::tree_hash::TreeHash<H> for $ref_name<'a> {
            fn tree_hash_type() -> ::tree_hash::TreeHashType {
                <$owned as ::tree_hash::TreeHash<H>>::tree_hash_type()
            }

            fn tree_hash_packed_encoding(&self) -> ::tree_hash::PackedEncoding {
                <$owned as ::tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.inner)
            }

            fn tree_hash_packing_factor() -> usize {
                <$owned as ::tree_hash::TreeHash<H>>::tree_hash_packing_factor()
            }

            fn tree_hash_root(&self) -> H::Output {
                <$owned as ::tree_hash::TreeHash<H>>::tree_hash_root(&self.inner)
            }
        }

        impl<'a> ::ssz_types::view::ToOwnedSsz<$owned> for $ref_name<'a> {
            fn to_owned(&self) -> $owned {
                self.inner
            }
        }
    };
}

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
