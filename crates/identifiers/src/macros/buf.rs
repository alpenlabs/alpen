use serde::de;

/// Decodes a hex string (with optional `0x`/`0X` prefix) into a fixed-size byte array.
///
/// If `reverse` is `true`, the decoded bytes are reversed in place (matching
/// Bitcoin's display convention where hashes are shown in reversed byte order).
pub(crate) fn decode_hex_to_array<const N: usize, E: de::Error>(
    v: &str,
    reverse: bool,
) -> Result<[u8; N], E> {
    let hex_str = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .unwrap_or(v);

    let bytes = hex::decode(hex_str).map_err(E::custom)?;

    if bytes.len() != N {
        return Err(E::custom(format!(
            "expected {} bytes, got {}",
            N,
            bytes.len()
        )));
    }

    let mut array = [0u8; N];
    array.copy_from_slice(&bytes);
    if reverse {
        array.reverse();
    }
    Ok(array)
}

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

/// Generates `Serialize` and `Deserialize` impls for a fixed-size byte buffer.
///
/// Human-readable formats (e.g. JSON) serialize as hex strings. When
/// `reverse_human_readable` is `true`, the byte order is reversed before/after
/// hex encoding, matching Bitcoin's display convention.
///
/// Non-human-readable formats (e.g. bincode) serialize as raw bytes, never
/// reversed.
macro_rules! impl_buf_serde_inner {
    ($name:ident, $len:expr, reverse_human_readable: $reverse:expr) => {
        impl ::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                if serializer.is_human_readable() {
                    if $reverse {
                        let mut bytes = self.0;
                        bytes.reverse();
                        serializer.serialize_str(&::hex::encode(&bytes))
                    } else {
                        serializer.serialize_str(&::hex::encode(&self.0))
                    }
                } else {
                    serializer.serialize_bytes(&self.0)
                }
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                struct BufVisitor(bool);

                impl<'de> ::serde::de::Visitor<'de> for BufVisitor {
                    type Value = $name;

                    fn expecting(
                        &self,
                        formatter: &mut ::std::fmt::Formatter<'_>,
                    ) -> ::std::fmt::Result {
                        write!(formatter, "a hex string or byte array of {} bytes", $len)
                    }

                    fn visit_str<E>(self, v: &str) -> Result<$name, E>
                    where
                        E: ::serde::de::Error,
                    {
                        $crate::macros::buf::decode_hex_to_array::<$len, E>(v, self.0)
                            .map($name)
                    }

                    fn visit_bytes<E>(self, v: &[u8]) -> Result<$name, E>
                    where
                        E: ::serde::de::Error,
                    {
                        let v: &[u8; $len] = v.try_into().map_err(E::custom)?;
                        Ok($name(*v))
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<$name, A::Error>
                    where
                        A: ::serde::de::SeqAccess<'de>,
                    {
                        let mut array = [0u8; $len];
                        for i in 0..$len {
                            array[i] = seq
                                .next_element::<u8>()?
                                .ok_or_else(|| {
                                    ::serde::de::Error::invalid_length(i, &self)
                                })?;
                        }
                        Ok($name(array))
                    }
                }

                if deserializer.is_human_readable() {
                    // `deserialize_any` so we accept both hex strings and
                    // JSON arrays.
                    deserializer.deserialize_any(BufVisitor($reverse))
                } else {
                    deserializer.deserialize_bytes(BufVisitor(false))
                }
            }
        }
    };
}

macro_rules! impl_buf_serde {
    ($name:ident, $len:expr) => {
        $crate::macros::buf::impl_buf_serde_inner!($name, $len, reverse_human_readable: false);
    };
}

/// Generates `Debug` (full reversed hex) and `Display` (truncated reversed hex) formatting.
///
/// Same as [`impl_buf_fmt`] but reverses the byte order before hex encoding,
/// matching Bitcoin's display convention for block/transaction hashes.
macro_rules! impl_rbuf_fmt {
    ($name:ident, $len:expr) => {
        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let mut bytes = self.0;
                bytes.reverse();
                let mut buf = [0; $len * 2];
                ::hex::encode_to_slice(bytes, &mut buf).expect("buf: enc hex");
                f.write_str(unsafe { ::core::str::from_utf8_unchecked(&buf) })
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let mut bytes = self.0;
                bytes.reverse();
                // fmt only first and last bits of the reversed data.
                let mut buf = [0; 6];
                ::hex::encode_to_slice(&bytes[..3], &mut buf).expect("buf: enc hex");
                f.write_str(unsafe { ::core::str::from_utf8_unchecked(&buf) })?;
                f.write_str("..")?;
                ::hex::encode_to_slice(&bytes[$len - 3..], &mut buf).expect("buf: enc hex");
                f.write_str(unsafe { ::core::str::from_utf8_unchecked(&buf) })?;
                Ok(())
            }
        }
    };
}

/// Generates reversed-byte `Serialize` and `Deserialize` impls.
///
/// Same as [`impl_buf_serde`] but reverses the byte order for human-readable formats,
/// matching Bitcoin's display convention for block/transaction hashes. Binary formats
/// (e.g. bincode) are unaffected and use raw byte order.
macro_rules! impl_rbuf_serde {
    ($name:ident, $len:expr) => {
        $crate::macros::buf::impl_buf_serde_inner!($name, $len, reverse_human_readable: true);
    };
}

pub(crate) use impl_buf_arbitrary;
pub(crate) use impl_buf_borsh;
pub(crate) use impl_buf_codec;
pub(crate) use impl_buf_core;
pub(crate) use impl_buf_fmt;
pub(crate) use impl_buf_serde;
pub(crate) use impl_buf_serde_inner;
pub(crate) use impl_rbuf_fmt;
pub(crate) use impl_rbuf_serde;
