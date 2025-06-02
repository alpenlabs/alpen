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
    };
}

pub mod internal {
    // Crate-internal impls.

    macro_rules! impl_buf_common {
        ($name:ident, $len:expr) => {
            impl $name {
                pub const LEN: usize = $len;

                pub fn new(data: [u8; $len]) -> Self {
                    Self(data)
                }

                pub fn as_slice(&self) -> &[u8] {
                    &self.0
                }

                pub fn as_mut_slice(&mut self) -> &mut [u8] {
                    &mut self.0
                }

                pub fn as_bytes(&self) -> &[u8] {
                    self.0.as_slice()
                }

                pub fn zero() -> Self {
                    Self([0; $len].into())
                }

                pub fn is_zero(&self) -> bool {
                    self.0.iter().all(|v| *v == 0)
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

            impl<'a> ::arbitrary::Arbitrary<'a> for $name {
                fn arbitrary(u: &mut ::arbitrary::Unstructured<'a>) -> ::arbitrary::Result<Self> {
                    let mut array = [0u8; $len];
                    u.fill_buffer(&mut array)?;
                    Ok(array.into())
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
                            formatter: &mut ::std::fmt::Formatter,
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

    pub(crate) use impl_buf_common;
    pub(crate) use impl_buf_serde;
}

#[cfg(test)]
mod tests {

    #[derive(PartialEq)]
    pub struct TestBuf20([u8; 20]);

    crate::macros::internal::impl_buf_common!(TestBuf20, 20);
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
}
