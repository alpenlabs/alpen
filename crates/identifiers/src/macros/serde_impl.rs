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
                        $crate::macros::serde_impl::decode_hex_to_array::<$len, E>(v, self.0)
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
                                .ok_or_else(|| ::serde::de::Error::invalid_length(i, &self))?;
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

/// Generates reversed-byte `Serialize` and `Deserialize` impls.
///
/// Reverses the byte order for human-readable formats, matching Bitcoin's display
/// convention for block/transaction hashes. Binary formats (e.g. bincode) are
/// unaffected and use raw byte order.
macro_rules! impl_rbuf_serde {
    ($name:ident, $len:expr) => {
        impl_buf_serde_inner!($name, $len, reverse_human_readable: true);
    };
}

pub(crate) use impl_rbuf_serde;
