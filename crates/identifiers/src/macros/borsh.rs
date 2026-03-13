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
