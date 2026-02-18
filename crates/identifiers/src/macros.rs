/// Generates impls for shims wrapping a type as another.
///
/// This must be a newtype a la `struct Foo(Bar);`.
#[macro_export]
macro_rules! impl_opaque_thin_wrapper {
    ($target:ty => $inner:ty) => {
        impl $target {
            pub const fn new(v: $inner) -> Self {
                Self(v)
            }

            pub fn inner(&self) -> &$inner {
                &self.0
            }

            pub fn into_inner(self) -> $inner {
                self.0
            }
        }

        $crate::strata_codec::impl_wrapper_codec!($target => $inner);

        impl From<$inner> for $target {
            fn from(value: $inner) -> $target {
                <$target>::new(value)
            }
        }

        impl From<$target> for $inner {
            fn from(value: $target) -> $inner {
                value.into_inner()
            }
        }
    };
}

/// Generates impls for shims wrapping a type as another, but where this is a
/// transparent relationship.
///
/// This must be a newtype a la `struct Foo(Bar);`.
#[macro_export]
macro_rules! impl_transparent_thin_wrapper {
    ($target:ty => $inner:ty) => {
        $crate::impl_opaque_thin_wrapper! { $target => $inner }

        impl std::ops::Deref for $target {
            type Target = $inner;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::ops::DerefMut for $target {
            fn deref_mut(&mut self) -> &mut $inner {
                &mut self.0
            }
        }
    };
}

#[macro_export]
macro_rules! impl_buf_wrapper {
    ($wrapper:ident, $name:ident, $len:expr) => {
        impl ::std::convert::From<$name> for $wrapper {
            fn from(value: $name) -> Self {
                Self(value)
            }
        }

        impl ::std::convert::From<$wrapper> for $name {
            fn from(value: $wrapper) -> Self {
                value.0
            }
        }

        impl ::std::convert::AsRef<[u8; $len]> for $wrapper {
            fn as_ref(&self) -> &[u8; $len] {
                self.0.as_ref()
            }
        }

        impl ::core::fmt::Debug for $wrapper {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Debug::fmt(&self.0, f)
            }
        }

        impl ::core::fmt::Display for $wrapper {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        // Codec implementation for Buf wrapper types - passthrough to underlying Buf
        impl $crate::strata_codec::Codec for $wrapper {
            fn encode(
                &self,
                enc: &mut impl $crate::strata_codec::Encoder,
            ) -> Result<(), $crate::strata_codec::CodecError> {
                // Delegate to the underlying Buf type's Codec implementation
                self.0.encode(enc)
            }

            fn decode(
                dec: &mut impl $crate::strata_codec::Decoder,
            ) -> Result<Self, $crate::strata_codec::CodecError> {
                // Decode the underlying Buf type and wrap it
                let buf = $name::decode(dec)?;
                Ok(Self(buf))
            }
        }
    };
}

pub(crate) mod internal {
    // Crate-internal impls.

    /// Generates the foundational API for a fixed-size byte buffer type.
    ///
    /// Provides constructors (`new`, `zero`), accessors (`as_slice`, `as_mut_slice`,
    /// `as_bytes`, `is_zero`), the `LEN` constant, standard conversion traits (`AsRef`,
    /// `AsMut`, `From`, `TryFrom`), and `Default`.
    macro_rules! impl_buf_core {
        ($name:ident, $len:expr) => {
            impl $name {
                pub const LEN: usize = $len;

                pub const fn new(data: [u8; $len]) -> Self {
                    Self(data)
                }

                pub const fn as_slice(&self) -> &[u8] {
                    &self.0
                }

                pub const fn as_mut_slice(&mut self) -> &mut [u8] {
                    &mut self.0
                }

                pub const fn as_bytes(&self) -> &[u8] {
                    self.0.as_slice()
                }

                pub const fn zero() -> Self {
                    Self::new([0; $len])
                }

                pub const fn is_zero(&self) -> bool {
                    let mut i = 0;
                    while i < $len {
                        if self.0[i] != 0 {
                            return false;
                        }
                        i += 1;
                    }
                    true
                }
            }

            impl ::std::convert::AsRef<[u8; $len]> for $name {
                fn as_ref(&self) -> &[u8; $len] {
                    &self.0
                }
            }

            impl ::std::convert::AsMut<[u8]> for $name {
                fn as_mut(&mut self) -> &mut [u8] {
                    &mut self.0
                }
            }

            impl ::std::convert::From<[u8; $len]> for $name {
                fn from(data: [u8; $len]) -> Self {
                    Self(data)
                }
            }

            impl ::std::convert::From<$name> for [u8; $len] {
                fn from(buf: $name) -> Self {
                    buf.0
                }
            }

            impl<'a> ::std::convert::From<&'a [u8; $len]> for $name {
                fn from(data: &'a [u8; $len]) -> Self {
                    Self(*data)
                }
            }

            impl<'a> ::std::convert::TryFrom<&'a [u8]> for $name {
                type Error = &'a [u8];

                fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
                    if value.len() == $len {
                        let mut arr = [0; $len];
                        arr.copy_from_slice(value);
                        Ok(Self(arr))
                    } else {
                        Err(value)
                    }
                }
            }

            impl ::std::default::Default for $name {
                fn default() -> Self {
                    Self([0; $len])
                }
            }
        };
    }

    /// Generates `Debug` (full hex) and `Display` (truncated hex) formatting.
    macro_rules! impl_buf_fmt {
        ($name:ident, $len:expr) => {
            impl ::std::fmt::Debug for $name {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    // twice as large, required by the hex::encode_to_slice.
                    let mut buf = [0; $len * 2];
                    ::hex::encode_to_slice(self.0, &mut buf).expect("buf: enc hex");
                    f.write_str(unsafe { ::core::str::from_utf8_unchecked(&buf) })
                }
            }

            impl ::std::fmt::Display for $name {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    // fmt only first and last bits of data.
                    let mut buf = [0; 6];
                    ::hex::encode_to_slice(&self.0[..3], &mut buf).expect("buf: enc hex");
                    f.write_str(unsafe { ::core::str::from_utf8_unchecked(&buf) })?;
                    f.write_str("..")?;
                    ::hex::encode_to_slice(&self.0[$len - 3..], &mut buf).expect("buf: enc hex");
                    f.write_str(unsafe { ::core::str::from_utf8_unchecked(&buf) })?;
                    Ok(())
                }
            }
        };
    }

    /// Generates `BorshSerialize` and `BorshDeserialize` impls.
    macro_rules! impl_buf_borsh {
        ($name:ident, $len:expr) => {
            impl ::borsh::BorshSerialize for $name {
                fn serialize<W: ::std::io::Write>(&self, writer: &mut W) -> ::std::io::Result<()> {
                    let bytes = self.0.as_ref();
                    let _ = writer.write(bytes)?;
                    Ok(())
                }
            }

            impl ::borsh::BorshDeserialize for $name {
                fn deserialize_reader<R: ::std::io::Read>(
                    reader: &mut R,
                ) -> ::std::io::Result<Self> {
                    let mut array = [0u8; $len];
                    reader.read_exact(&mut array)?;
                    Ok(array.into())
                }
            }
        };
    }

    /// Generates `Arbitrary` impl for property-based testing.
    macro_rules! impl_buf_arbitrary {
        ($name:ident, $len:expr) => {
            impl<'a> ::arbitrary::Arbitrary<'a> for $name {
                fn arbitrary(u: &mut ::arbitrary::Unstructured<'a>) -> ::arbitrary::Result<Self> {
                    let mut array = [0u8; $len];
                    u.fill_buffer(&mut array)?;
                    Ok(array.into())
                }
            }
        };
    }

    /// Generates `strata_codec::Codec` impl.
    macro_rules! impl_buf_codec {
        ($name:ident, $len:expr) => {
            impl $crate::strata_codec::Codec for $name {
                fn encode(
                    &self,
                    enc: &mut impl $crate::strata_codec::Encoder,
                ) -> Result<(), $crate::strata_codec::CodecError> {
                    self.0.encode(enc)
                }

                fn decode(
                    dec: &mut impl $crate::strata_codec::Decoder,
                ) -> Result<Self, $crate::strata_codec::CodecError> {
                    let bytes = <[u8; $len]>::decode(dec)?;
                    Ok(Self(bytes))
                }
            }
        };
    }

    macro_rules! impl_buf_serde {
        ($name:ident, $len:expr) => {
            impl ::serde::Serialize for $name {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: ::serde::Serializer,
                {
                    // Convert the inner array to a hex string (without 0x prefix)
                    let hex_str = ::hex::encode(&self.0);
                    serializer.serialize_str(&hex_str)
                }
            }

            impl<'de> ::serde::Deserialize<'de> for $name {
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: ::serde::Deserializer<'de>,
                {
                    // Define a Visitor for deserialization.
                    // P.S. Make it in the scope of the function to avoid name conflicts
                    // for different macro_rules invocations.
                    struct BufVisitor;

                    impl<'de> ::serde::de::Visitor<'de> for BufVisitor {
                        type Value = $name;

                        fn expecting(
                            &self,
                            formatter: &mut ::std::fmt::Formatter<'_>,
                        ) -> ::std::fmt::Result {
                            write!(
                                formatter,
                                "a hex string with an optional 0x prefix representing {} bytes",
                                $len
                            )
                        }

                        fn visit_str<E>(self, v: &str) -> Result<$name, E>
                        where
                            E: ::serde::de::Error,
                        {
                            // Remove the optional "0x" or "0X" prefix if present.
                            let hex_str = if v.starts_with("0x") || v.starts_with("0X") {
                                &v[2..]
                            } else {
                                v
                            };

                            // Decode the hex string into a vector of bytes.
                            let bytes = ::hex::decode(hex_str).map_err(E::custom)?;

                            // Ensure the decoded bytes have the expected length.
                            if bytes.len() != $len {
                                return Err(E::custom(format!(
                                    "expected {} bytes, got {}",
                                    $len,
                                    bytes.len()
                                )));
                            }

                            // Convert the Vec<u8> into a fixed-size array.
                            let mut array = [0u8; $len];
                            array.copy_from_slice(&bytes);
                            Ok($name(array))
                        }

                        fn visit_bytes<E>(self, v: &[u8]) -> Result<$name, E>
                        where
                            E: ::serde::de::Error,
                        {
                            if v.len() == $len {
                                let mut array = [0u8; $len];
                                array.copy_from_slice(v);
                                Ok($name(array))
                            } else {
                                // Try to interpret the bytes as a UTF-8 encoded hex string.
                                let s = ::std::str::from_utf8(v).map_err(E::custom)?;
                                self.visit_str(s)
                            }
                        }

                        fn visit_seq<A>(self, mut seq: A) -> Result<$name, A::Error>
                        where
                            A: ::serde::de::SeqAccess<'de>,
                        {
                            let mut array = [0u8; $len];
                            for i in 0..$len {
                                array[i] = seq
                                    .next_element::<u8>()?
                                    .ok_or_else(|| ::serde::de::Error::invalid_length(i, &self))?;
                            }
                            // Ensure there are no extra elements.
                            if let Some(_) = seq.next_element::<u8>()? {
                                return Err(::serde::de::Error::custom(format!(
                                    "expected a sequence of exactly {} bytes, but found extra elements",
                                    $len
                                )));
                            }
                            Ok($name(array))
                        }
                    }

                    if deserializer.is_human_readable() {
                        // For human-readable formats, support multiple input types.
                        // Use with the _any, so serde can decide whether to visit seq, bytes or str.
                        deserializer.deserialize_any(BufVisitor)
                    } else {
                        // Bincode does not support DeserializeAny, so deserializing with the _str.
                        deserializer.deserialize_str(BufVisitor)
                    }
                }
            }
        };
    }

    pub(crate) use impl_buf_arbitrary;
    pub(crate) use impl_buf_borsh;
    pub(crate) use impl_buf_codec;
    pub(crate) use impl_buf_core;
    pub(crate) use impl_buf_fmt;
    pub(crate) use impl_buf_serde;
}

/// Implements Borsh serialization as a shim over SSZ bytes with length-prefixing.
///
/// This macro generates BorshSerialize and BorshDeserialize implementations that:
/// 1. Convert the type to/from SSZ bytes
/// 2. Use length-prefixed encoding (u32 length followed by data) to support nested structs
///
/// This solves the issue where `read_to_end()` fails when types are embedded in other structs,
/// because it consumes the entire remaining stream. The length-prefix approach reads exactly
/// the number of bytes needed for this specific value.
///
/// # Requirements
///
/// The type must implement both `ssz::Encode` and `ssz::Decode` traits.
///
/// # Example
///
/// ```ignore
/// use ssz_derive::{Decode, Encode};
///
/// #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
/// pub struct MyType {
///     field: u64,
/// }
///
/// impl_borsh_via_ssz!(MyType);
/// ```
#[macro_export]
macro_rules! impl_borsh_via_ssz {
    ($type:ty) => {
        impl ::borsh::BorshSerialize for $type {
            fn serialize<W: ::std::io::Write>(&self, writer: &mut W) -> ::std::io::Result<()> {
                // Convert to SSZ bytes
                let bytes = ::ssz::Encode::as_ssz_bytes(self);

                // Write length as u32 (Borsh standard)
                let len = bytes.len() as u32;
                writer.write_all(&len.to_le_bytes())?;

                // Write the SSZ bytes
                writer.write_all(&bytes)?;

                Ok(())
            }
        }

        impl ::borsh::BorshDeserialize for $type {
            fn deserialize_reader<R: ::std::io::Read>(reader: &mut R) -> ::std::io::Result<Self> {
                // Read length as u32 (Borsh standard)
                let mut len_bytes = [0u8; 4];
                reader.read_exact(&mut len_bytes)?;
                let len = u32::from_le_bytes(len_bytes) as usize;

                // Read exactly len bytes
                let mut buffer = vec![0u8; len];
                reader.read_exact(&mut buffer)?;

                // Decode from SSZ bytes
                ::ssz::Decode::from_ssz_bytes(&buffer).map_err(|e| {
                    ::std::io::Error::new(
                        ::std::io::ErrorKind::InvalidData,
                        format!("SSZ decode error: {:?}", e),
                    )
                })
            }
        }
    };
}

/// Implements Borsh serialization as a shim over SSZ bytes for fixed-size types.
///
/// This macro generates BorshSerialize and BorshDeserialize implementations that:
/// 1. Convert the type to/from SSZ bytes
/// 2. Write/read SSZ bytes directly WITHOUT length-prefixing (for fixed-size types)
///
/// Use this macro for commitment types and other fixed-size SSZ containers where the size
/// is always known. For variable-length types that may be nested, use `impl_borsh_via_ssz!`
/// instead (which adds length-prefixing).
///
/// # Requirements
///
/// The type must:
/// - Implement both `ssz::Encode` and `ssz::Decode` traits
/// - Be a fixed-size SSZ container (ssz_fixed_len() returns true)
///
/// # Example
///
/// ```ignore
/// use ssz_derive::{Decode, Encode};
///
/// #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
/// pub struct OLBlockCommitment {
///     slot: u64,
///     blkid: OLBlockId,
/// }
///
/// impl_borsh_via_ssz_fixed!(OLBlockCommitment);
/// ```
#[macro_export]
macro_rules! impl_borsh_via_ssz_fixed {
    ($type:ty) => {
        impl ::borsh::BorshSerialize for $type {
            fn serialize<W: ::std::io::Write>(&self, writer: &mut W) -> ::std::io::Result<()> {
                // Convert to SSZ bytes and write directly (no length prefix)
                let ssz_bytes = ::ssz::Encode::as_ssz_bytes(self);
                writer.write_all(&ssz_bytes)
            }
        }

        impl ::borsh::BorshDeserialize for $type {
            fn deserialize_reader<R: ::std::io::Read>(reader: &mut R) -> ::std::io::Result<Self> {
                // Read exactly the SSZ fixed length
                // This is critical: we must read exactly the fixed length, not all remaining bytes,
                // because this type may be nested inside larger Borsh structures.
                let ssz_fixed_len = <$type as ::ssz::Decode>::ssz_fixed_len();
                let mut ssz_bytes = vec![0u8; ssz_fixed_len];
                reader.read_exact(&mut ssz_bytes)?;

                // Decode from SSZ bytes
                ::ssz::Decode::from_ssz_bytes(&ssz_bytes).map_err(|e| {
                    ::std::io::Error::new(
                        ::std::io::ErrorKind::InvalidData,
                        format!("SSZ decode error: {:?}", e),
                    )
                })
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

#[cfg(test)]
mod tests {

    #[derive(PartialEq)]
    pub struct TestBuf20([u8; 20]);

    crate::macros::internal::impl_buf_core!(TestBuf20, 20);
    crate::macros::internal::impl_buf_fmt!(TestBuf20, 20);
    crate::macros::internal::impl_buf_borsh!(TestBuf20, 20);
    crate::macros::internal::impl_buf_arbitrary!(TestBuf20, 20);
    crate::macros::internal::impl_buf_codec!(TestBuf20, 20);
    crate::macros::internal::impl_buf_serde!(TestBuf20, 20);

    #[test]
    fn test_from_into_array() {
        let buf = TestBuf20::new([5u8; 20]);
        let arr: [u8; 20] = buf.into();
        assert_eq!(arr, [5; 20]);
    }

    #[test]
    fn test_from_array_ref() {
        let arr = [2u8; 20];
        let buf: TestBuf20 = TestBuf20::from(&arr);
        assert_eq!(buf.as_slice(), &arr);
    }

    #[test]
    fn test_default() {
        let buf = TestBuf20::default();
        assert_eq!(buf.as_slice(), &[0; 20]);
    }

    #[test]
    fn test_serialize_hex() {
        let data = [1u8; 20];
        let buf = TestBuf20(data);
        let json = serde_json::to_string(&buf).unwrap();
        // Since we serialize as a string, json should be the hex-encoded string wrapped in quotes.
        let expected = format!("\"{}\"", hex::encode(data));
        assert_eq!(json, expected);
    }

    #[test]
    fn test_deserialize_hex_without_prefix() {
        let data = [2u8; 20];
        let hex_str = hex::encode(data);
        let json = format!("\"{hex_str}\"");
        let buf: TestBuf20 = serde_json::from_str(&json).unwrap();
        assert_eq!(buf, TestBuf20(data));
    }

    #[test]
    fn test_deserialize_hex_with_prefix() {
        let data = [3u8; 20];
        let hex_str = hex::encode(data);
        let json = format!("\"0x{hex_str}\"");
        let buf: TestBuf20 = serde_json::from_str(&json).unwrap();
        assert_eq!(buf, TestBuf20(data));
    }

    #[test]
    fn test_deserialize_from_seq() {
        // Provide a JSON array of numbers.
        let data = [5u8; 20];
        let json = serde_json::to_string(&data).unwrap();
        let buf: TestBuf20 = serde_json::from_str(&json).unwrap();
        assert_eq!(buf, TestBuf20(data));
    }

    #[test]
    fn test_deserialize_from_bytes_via_array() {
        // Although JSON doesn't have a native "bytes" type, this test uses a JSON array
        // to exercise the same code path as visit_bytes when deserializing a sequence.
        let data = [7u8; 20];
        // Simulate input as a JSON array
        let json = serde_json::to_string(&data).unwrap();
        let buf: TestBuf20 = serde_json::from_str(&json).unwrap();
        assert_eq!(buf, TestBuf20(data));
    }

    #[test]
    fn test_bincode_roundtrip() {
        let data = [9u8; 20];
        let buf = TestBuf20(data);
        // bincode is non-human-readable so our implementation will use deserialize_tuple.
        let encoded = bincode::serialize(&buf).expect("bincode serialization failed");
        let decoded: TestBuf20 =
            bincode::deserialize(&encoded).expect("bincode deserialization failed");
        assert_eq!(buf, decoded);
    }

    use std::io;

    // Test the SSZ transparent wrapper macros
    use ssz::{Decode, Encode};
    use ssz_derive::{Decode, Encode};

    use crate::buf::Buf32;

    #[derive(Copy, Clone, Debug, Eq, PartialEq, Encode, Decode)]
    #[ssz(struct_behaviour = "transparent")]
    struct TestBuf32Wrapper(Buf32);

    crate::impl_ssz_transparent_buf32_wrapper_copy!(TestBuf32Wrapper);

    #[test]
    fn test_ssz_transparent_wrapper_roundtrip() {
        let data = [42u8; 32];
        let wrapper = TestBuf32Wrapper(Buf32::new(data));

        // Test SSZ encoding/decoding
        let encoded = wrapper.as_ssz_bytes();
        let decoded = TestBuf32Wrapper::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(wrapper, decoded);
    }

    #[test]
    fn test_ssz_transparent_wrapper_tree_hash() {
        use tree_hash::{Sha256Hasher, TreeHash};

        let data = [42u8; 32];
        let wrapper = TestBuf32Wrapper(Buf32::new(data));
        let inner = Buf32::new(data);

        // TreeHash should be the same as inner type (transparent)
        let wrapper_hash = TreeHash::<Sha256Hasher>::tree_hash_root(&wrapper);
        let inner_hash = TreeHash::<Sha256Hasher>::tree_hash_root(&inner);
        assert_eq!(wrapper_hash, inner_hash);
    }

    #[test]
    fn test_ssz_transparent_wrapper_to_owned() {
        use ssz_types::view::ToOwnedSsz;

        let data = [42u8; 32];
        let wrapper = TestBuf32Wrapper(Buf32::new(data));

        // ToOwnedSsz should return a copy
        let owned = ToOwnedSsz::to_owned(&wrapper);
        assert_eq!(wrapper, owned);
    }

    // Test the Borsh-via-SSZ macro
    #[derive(Clone, Debug, Eq, PartialEq, Encode, Decode)]
    struct TestBorshViaSsz {
        value: u64,
        data: Vec<u8>,
    }

    crate::impl_borsh_via_ssz!(TestBorshViaSsz);

    #[test]
    fn test_borsh_via_ssz_roundtrip() {
        use borsh::{BorshDeserialize, BorshSerialize};

        let original = TestBorshViaSsz {
            value: 42,
            data: vec![1, 2, 3, 4, 5],
        };

        // Test Borsh serialization roundtrip
        let mut buffer = Vec::new();
        original.serialize(&mut buffer).unwrap();

        let decoded = TestBorshViaSsz::deserialize_reader(&mut buffer.as_slice()).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_borsh_via_ssz_nested() {
        use borsh::{BorshDeserialize, BorshSerialize};

        // Test that our length-prefixed approach works when nested
        #[derive(Clone, Debug, Eq, PartialEq)]
        struct Container {
            first: TestBorshViaSsz,
            second: TestBorshViaSsz,
        }

        impl borsh::BorshSerialize for Container {
            fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
                self.first.serialize(writer)?;
                self.second.serialize(writer)?;
                Ok(())
            }
        }

        impl borsh::BorshDeserialize for Container {
            fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
                let first = TestBorshViaSsz::deserialize_reader(reader)?;
                let second = TestBorshViaSsz::deserialize_reader(reader)?;
                Ok(Container { first, second })
            }
        }

        let container = Container {
            first: TestBorshViaSsz {
                value: 100,
                data: vec![1, 2, 3],
            },
            second: TestBorshViaSsz {
                value: 200,
                data: vec![4, 5, 6, 7],
            },
        };

        // Serialize and deserialize
        let mut buffer = Vec::new();
        container.serialize(&mut buffer).unwrap();

        let decoded = Container::deserialize_reader(&mut buffer.as_slice()).unwrap();
        assert_eq!(container.first, decoded.first);
        assert_eq!(container.second, decoded.second);
    }

    // Test the fixed-size Borsh-via-SSZ macro
    #[test]
    fn test_borsh_via_ssz_fixed() {
        use borsh::{BorshDeserialize, BorshSerialize};

        use crate::{Buf32, EpochCommitment, OLBlockCommitment, OLBlockId};

        // Test OLBlockCommitment - should be 40 bytes, no length prefix
        let commitment = OLBlockCommitment::new(12345, OLBlockId::from(Buf32::from([42u8; 32])));

        let mut buffer = Vec::new();
        commitment.serialize(&mut buffer).unwrap();

        // Should be exactly 40 bytes (8 for slot + 32 for blkid), no length prefix
        assert_eq!(buffer.len(), 40, "OLBlockCommitment should be 40 bytes");

        // First 8 bytes should be the slot in little-endian
        let slot_bytes = 12345u64.to_le_bytes();
        assert_eq!(&buffer[0..8], &slot_bytes, "First 8 bytes should be slot");

        // Next 32 bytes should be the blkid
        assert_eq!(&buffer[8..40], &[42u8; 32], "Next 32 bytes should be blkid");

        // Test deserialization
        let decoded = OLBlockCommitment::deserialize_reader(&mut buffer.as_slice()).unwrap();
        assert_eq!(decoded.slot(), 12345);
        assert_eq!(decoded.blkid().as_ref(), &[42u8; 32]);

        // Test EpochCommitment - should be 44 bytes, no length prefix
        let epoch_commitment =
            EpochCommitment::new(5, 100, OLBlockId::from(Buf32::from([99u8; 32])));

        let mut buffer2 = Vec::new();
        epoch_commitment.serialize(&mut buffer2).unwrap();

        // Should be exactly 44 bytes (4 for epoch + 8 for slot + 32 for blkid), no length prefix
        assert_eq!(buffer2.len(), 44, "EpochCommitment should be 44 bytes");

        // Verify no length prefix by checking first 4 bytes are the epoch, not a length
        let epoch_bytes = 5u32.to_le_bytes();
        assert_eq!(
            &buffer2[0..4],
            &epoch_bytes,
            "First 4 bytes should be epoch"
        );

        // Test deserialization
        let decoded2 = EpochCommitment::deserialize_reader(&mut buffer2.as_slice()).unwrap();
        assert_eq!(decoded2.epoch(), 5);
        assert_eq!(decoded2.last_slot(), 100);
    }
}
