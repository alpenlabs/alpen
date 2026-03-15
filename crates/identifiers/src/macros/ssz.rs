//! SSZ view trait macros for identifier and wrapper types.
//!
//! These macros generate the boilerplate trait implementations (`DecodeView`,
//! `SszTypeInfo`, `TreeHash`, `ToOwnedSsz`) that the SSZ view layer requires.
//! Three macros are provided, each targeting a different structural pattern:
//!
//! | Macro | Use when… |
//! |---|---|
//! | [`impl_ssz_fixed_container!`] | Multi-field struct with `#[ssz(struct_behaviour = "container")]` |
//! | [`impl_ssz_transparent_wrapper!`] | Newtype whose inner type already implements `DecodeView` |
//! | [`impl_ssz_transparent_byte_array_wrapper!`] | Newtype wrapping a raw `[u8; N]` (which lacks `DecodeView`) |
//!
//! ## Choosing the right macro
//!
//! ```text
//!  Is the type a multi-field container?
//!    ├─ Yes → impl_ssz_fixed_container!
//!    └─ No (newtype / transparent wrapper)
//!         ├─ Inner type has DecodeView? (Buf32, RBuf32, u64, …)
//!         │    └─ Yes → impl_ssz_transparent_wrapper!
//!         └─ Inner type is [u8; N]?
//!              └─ Yes → impl_ssz_transparent_byte_array_wrapper!
//! ```
//!
//! The split between the two transparent-wrapper macros exists because `[u8; N]`
//! does **not** implement `DecodeView` in the `ssz` crate — only
//! `FixedBytes<N>` does. Since both `[u8; N]` and `DecodeView` are foreign,
//! the orphan rule prevents adding that impl locally, so
//! `impl_ssz_transparent_byte_array_wrapper!` provides a manual `DecodeView`
//! via `TryInto` plus `From` conversions with `FixedBytes<N>`.

/// Generates SSZ view trait implementations for a fixed-size container type.
///
/// Use this for structs annotated with `#[ssz(struct_behaviour = "container")]`
/// whose fields are all fixed-size. Generates:
/// - Generic `TreeHash<H>` implementation
/// - `SszTypeInfo` implementation (computes fixed size from field types)
/// - A `{Type}Ref` view type with `DecodeView`, `SszTypeInfo`, `TreeHash`, and `ToOwnedSsz`
///
/// The `{Type}Ref` name is auto-generated via [`paste`].
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
/// impl_ssz_fixed_container!(MyContainer, [a: u64, b: Buf32]);
/// // Generates: MyContainerRef<'a>
/// ```
#[macro_export]
macro_rules! impl_ssz_fixed_container {
    ($type:ident, [$($field:ident: $field_ty:ty),+ $(,)?]) => {
        ::paste::paste! {
            // TreeHash implementation
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
                        $crate::impl_ssz_fixed_container!(@count $($field),+)
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

            // SszTypeInfo implementation
            impl ::ssz::view::SszTypeInfo for $type {
                fn is_ssz_fixed_len() -> bool {
                    true
                }

                fn ssz_fixed_len() -> usize {
                    0 $(+ <$field_ty as ::ssz::view::SszTypeInfo>::ssz_fixed_len())+
                }
            }

            // Ref view type
            #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
            pub struct [<$type Ref>]<'a> {
                inner: $type,
                _phantom: ::std::marker::PhantomData<&'a ()>,
            }

            impl<'a> ::ssz::view::DecodeView<'a> for [<$type Ref>]<'a> {
                fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ::ssz::DecodeError> {
                    let inner = <$type as ::ssz::Decode>::from_ssz_bytes(bytes)?;
                    Ok(Self {
                        inner,
                        _phantom: ::std::marker::PhantomData,
                    })
                }
            }

            impl<'a> ::ssz::view::SszTypeInfo for [<$type Ref>]<'a> {
                fn is_ssz_fixed_len() -> bool {
                    <$type as ::ssz::view::SszTypeInfo>::is_ssz_fixed_len()
                }

                fn ssz_fixed_len() -> usize {
                    <$type as ::ssz::view::SszTypeInfo>::ssz_fixed_len()
                }
            }

            impl<'a, H: ::tree_hash::TreeHashDigest> ::tree_hash::TreeHash<H> for [<$type Ref>]<'a> {
                fn tree_hash_type() -> ::tree_hash::TreeHashType {
                    <$type as ::tree_hash::TreeHash<H>>::tree_hash_type()
                }

                fn tree_hash_packed_encoding(&self) -> ::tree_hash::PackedEncoding {
                    <$type as ::tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.inner)
                }

                fn tree_hash_packing_factor() -> usize {
                    <$type as ::tree_hash::TreeHash<H>>::tree_hash_packing_factor()
                }

                fn tree_hash_root(&self) -> H::Output {
                    <$type as ::tree_hash::TreeHash<H>>::tree_hash_root(&self.inner)
                }
            }

            impl<'a> ::ssz_types::view::ToOwnedSsz<$type> for [<$type Ref>]<'a> {
                fn to_owned(&self) -> $type {
                    self.inner
                }
            }
        }
    };
    // Internal helper: count the number of fields
    (@count $head:ident $(, $tail:ident)*) => {
        1usize $(+ $crate::impl_ssz_fixed_container!(@count_one $tail))*
    };
    (@count_one $x:ident) => { 1usize };
}

/// Generates SSZ view trait implementations for transparent wrappers whose
/// inner type already implements `DecodeView`.
///
/// Use this for newtypes wrapping types like `Buf32`, `u64`, or other types
/// that already have `DecodeView`, `SszTypeInfo`, and `TreeHash` implementations.
/// All trait implementations delegate to the inner type.
///
/// For types wrapping raw `[u8; N]` arrays (which do *not* implement
/// `DecodeView`), use `impl_ssz_transparent_byte_array_wrapper!` instead.
///
/// # Example
///
/// ```ignore
/// #[derive(Copy, Clone, Eq, PartialEq, Encode, Decode)]
/// #[ssz(struct_behaviour = "transparent")]
/// pub struct OLBlockId(Buf32);
///
/// impl_ssz_transparent_wrapper!(OLBlockId, Buf32);
/// ```
#[macro_export]
macro_rules! impl_ssz_transparent_wrapper {
    ($wrapper:ty, $inner:ty) => {
        // Manual DecodeView implementation for transparent wrapper
        // Uses fully qualified path to avoid conflicts with Decode derive
        impl<'a> ::ssz::view::DecodeView<'a> for $wrapper {
            fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ::ssz::DecodeError> {
                Ok(Self(<$inner as ::ssz::view::DecodeView>::from_ssz_bytes(
                    bytes,
                )?))
            }
        }

        // SszTypeInfo implementation delegated to inner type
        impl ::ssz::view::SszTypeInfo for $wrapper {
            fn is_ssz_fixed_len() -> bool {
                <$inner as ::ssz::view::SszTypeInfo>::is_ssz_fixed_len()
            }

            fn ssz_fixed_len() -> usize {
                <$inner as ::ssz::view::SszTypeInfo>::ssz_fixed_len()
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

/// Generates SSZ view trait implementations for transparent wrappers around
/// raw `[u8; N]` arrays.
///
/// This exists because `[u8; N]` does not implement `DecodeView` in the `ssz`
/// crate (only `FixedBytes<N>` does), so [`impl_ssz_transparent_wrapper!`]
/// cannot be used. This macro provides:
/// - A manual `DecodeView` implementation via `bytes.try_into()`
/// - `SszTypeInfo` (fixed-length)
/// - `TreeHash` delegating to `[u8; N]`
/// - Bidirectional `From` conversions with `FixedBytes<N>` for SSZ codegen interop
///
/// # Example
///
/// ```ignore
/// #[derive(Copy, Clone, Eq, PartialEq, Encode, Decode)]
/// #[ssz(struct_behaviour = "transparent")]
/// pub struct Buf32(pub [u8; 32]);
///
/// impl_ssz_transparent_byte_array_wrapper!(Buf32, 32);
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

        // FixedBytes conversions for SSZ interop
        impl ::core::convert::From<::ssz_primitives::FixedBytes<$len>> for $wrapper {
            fn from(value: ::ssz_primitives::FixedBytes<$len>) -> Self {
                Self(value.0)
            }
        }

        impl ::core::convert::From<&::ssz_primitives::FixedBytes<$len>> for &$wrapper {
            fn from(value: &::ssz_primitives::FixedBytes<$len>) -> Self {
                // SAFETY: FixedBytes<N> and the wrapper have the same layout
                unsafe { &*(value as *const ::ssz_primitives::FixedBytes<$len> as *const $wrapper) }
            }
        }

        impl ::core::convert::From<$wrapper> for ::ssz_primitives::FixedBytes<$len> {
            fn from(value: $wrapper) -> Self {
                ::ssz_primitives::FixedBytes(value.0)
            }
        }

        impl ::core::convert::From<&$wrapper> for &::ssz_primitives::FixedBytes<$len> {
            fn from(value: &$wrapper) -> Self {
                // SAFETY: the wrapper and FixedBytes<N> have the same layout
                unsafe { &*(value as *const $wrapper as *const ::ssz_primitives::FixedBytes<$len>) }
            }
        }
    };
}
