#[macro_use]
pub(crate) mod borsh;
#[macro_use]
pub(crate) mod buf;
#[cfg(feature = "serde")]
#[macro_use]
pub(crate) mod serde_impl;
#[cfg(feature = "ssz")]
#[macro_use]
mod ssz;
#[macro_use]
mod wrapper;

#[cfg(test)]
mod tests {
    #[derive(PartialEq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    #[cfg_attr(
        feature = "borsh",
        derive(borsh::BorshSerialize, borsh::BorshDeserialize)
    )]
    #[cfg_attr(feature = "codec", derive(strata_codec::Codec))]
    pub struct TestBuf20(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] [u8; 20]);

    crate::macros::buf::impl_buf_core!(TestBuf20, 20);
    crate::macros::buf::impl_buf_fmt!(TestBuf20, 20);

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

    #[cfg(feature = "serde")]
    #[test]
    fn test_serialize_hex() {
        let data = [1u8; 20];
        let buf = TestBuf20(data);
        let json = serde_json::to_string(&buf).unwrap();
        // Since we serialize as a string, json should be the hex-encoded string wrapped in quotes.
        let expected = format!("\"{}\"", hex::encode(data));
        assert_eq!(json, expected);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_deserialize_hex_without_prefix() {
        let data = [2u8; 20];
        let hex_str = hex::encode(data);
        let json = format!("\"{hex_str}\"");
        let buf: TestBuf20 = serde_json::from_str(&json).unwrap();
        assert_eq!(buf, TestBuf20(data));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_bincode_roundtrip() {
        let data = [9u8; 20];
        let buf = TestBuf20(data);
        let encoded = bincode::serialize(&buf).expect("bincode serialization failed");
        let decoded: TestBuf20 =
            bincode::deserialize(&encoded).expect("bincode deserialization failed");
        assert_eq!(buf, decoded);
    }

    #[cfg(feature = "ssz")]
    mod ssz_wrapper_tests {
        use ssz::{Decode, Encode};
        use ssz_derive::{Decode, Encode};

        use crate::buf::Buf32;

        #[derive(Copy, Clone, Debug, Eq, PartialEq, Encode, Decode)]
        #[ssz(struct_behaviour = "transparent")]
        struct TestBuf32Wrapper(Buf32);

        crate::impl_ssz_transparent_wrapper!(TestBuf32Wrapper, Buf32);

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
    }

    #[cfg(all(feature = "borsh", feature = "ssz"))]
    mod borsh_tests {
        use std::io;

        use borsh::{BorshDeserialize, BorshSerialize};
        use ssz_derive::{Decode, Encode};

        // Test the Borsh-via-SSZ macro
        #[derive(Clone, Debug, Eq, PartialEq, Encode, Decode)]
        struct TestBorshViaSsz {
            value: u64,
            data: Vec<u8>,
        }

        crate::impl_borsh_via_ssz!(TestBorshViaSsz);

        #[test]
        fn test_borsh_via_ssz_roundtrip() {
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
            // Test that our length-prefixed approach works when nested
            #[derive(Clone, Debug, Eq, PartialEq)]
            struct Container {
                first: TestBorshViaSsz,
                second: TestBorshViaSsz,
            }

            impl BorshSerialize for Container {
                fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
                    self.first.serialize(writer)?;
                    self.second.serialize(writer)?;
                    Ok(())
                }
            }

            impl BorshDeserialize for Container {
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
            use crate::{Buf32, EpochCommitment, OLBlockCommitment, OLBlockId};

            // Test OLBlockCommitment - should be 40 bytes, no length prefix
            let commitment =
                OLBlockCommitment::new(12345, OLBlockId::from(Buf32::from([42u8; 32])));

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

            // Should be exactly 44 bytes (4 for epoch + 8 for slot + 32 for blkid), no length
            // prefix
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
}
