//! OL-owned framing around ASM's native SSZ encodings.
//!
//! Retained manifests, logs and checkpoints keep the release's Borsh framing.
//! State and auxiliary data have changed layouts and carry distinct version
//! markers. Legacy state/aux rows require an explicit reset and replay before
//! using this binary; decoding never converts or deletes database records.

use std::io::{self, Read};

use borsh::{BorshDeserialize, BorshSerialize};
use ssz::{Decode, Encode};
use strata_asm_common::AsmLogEntry;

pub(crate) const STATE_V1: &[u8; 8] = b"ASMS\0\0\0\x01";
pub(crate) const AUX_V1: &[u8; 8] = b"ASMA\0\0\0\x01";

pub(crate) fn encode_versioned<T: Encode>(value: &T, marker: &[u8; 8]) -> io::Result<Vec<u8>> {
    let mut bytes = marker.to_vec();
    value.ssz_append(&mut bytes);
    Ok(bytes)
}

pub(crate) fn decode_versioned<T: Decode>(bytes: &[u8], marker: &[u8; 8]) -> io::Result<T> {
    let payload = bytes.strip_prefix(marker.as_slice()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported ASM storage format; legacy state/aux rows require reset and replay",
        )
    })?;
    T::from_ssz_bytes(payload).map_err(invalid_ssz)
}

// The old impl_borsh_via_ssz encoded each variable-length SSZ value as Vec<u8>.
struct FramedSsz<T>(T);

impl<T: Decode> BorshDeserialize for FramedSsz<T> {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let bytes = Vec::<u8>::deserialize_reader(reader)?;
        T::from_ssz_bytes(&bytes).map(Self).map_err(invalid_ssz)
    }
}

pub(crate) fn encode_framed<T: Encode>(value: &T) -> io::Result<Vec<u8>> {
    borsh::to_vec(&value.as_ssz_bytes())
}

pub(crate) fn decode_framed<T: Decode>(bytes: &[u8]) -> io::Result<T> {
    borsh::from_slice::<FramedSsz<T>>(bytes).map(|value| value.0)
}

pub(crate) fn encode_logs(logs: &[AsmLogEntry]) -> io::Result<Vec<u8>> {
    let count =
        u32::try_from(logs.len()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut bytes = count.to_le_bytes().to_vec();
    for log in logs {
        BorshSerialize::serialize(&log.as_ssz_bytes(), &mut bytes)?;
    }
    Ok(bytes)
}

pub(crate) fn decode_logs(bytes: &[u8]) -> io::Result<Vec<AsmLogEntry>> {
    borsh::from_slice::<Vec<FramedSsz<AsmLogEntry>>>(bytes)
        .map(|logs| logs.into_iter().map(|log| log.0).collect())
}

fn invalid_ssz(error: ssz::DecodeError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid ASM SSZ: {error}"),
    )
}

macro_rules! impl_asm_value_codec {
    ($schema:ident, $value:ty, $encode:expr, $decode:expr) => {
        impl typed_sled::codec::ValueCodec<$schema> for $value {
            type Decoded = Self;

            fn encode_value(&self) -> Result<Vec<u8>, typed_sled::codec::CodecError> {
                ($encode)(self).map_err(|error| {
                    typed_sled::codec::CodecError::SerializationFailed {
                        schema: $schema::tree_name(),
                        source: error.into(),
                    }
                })
            }

            fn decode_value(data: sled::IVec) -> Result<Self, typed_sled::codec::CodecError> {
                ($decode)(data.as_ref()).map_err(|error| {
                    typed_sled::codec::CodecError::DeserializationFailed {
                        schema: $schema::tree_name(),
                        source: error.into(),
                    }
                })
            }
        }
    };
}

pub(crate) use impl_asm_value_codec;

#[cfg(test)]
mod tests {
    use strata_asm_common::{AnchorState, AsmManifest, AuxData};
    use strata_asm_proto_checkpoint_types::CheckpointPayload;
    use strata_asm_proto_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_db_tests::asm_tests::make_test_asm_state;

    use super::*;

    #[test]
    fn state_and_aux_roundtrip_with_distinct_versions() {
        let state = make_test_asm_state();
        let bytes = encode_versioned(state.state(), STATE_V1).unwrap();
        assert_eq!(
            decode_versioned::<AnchorState>(&bytes, STATE_V1).unwrap(),
            *state.state()
        );
        assert!(decode_versioned::<AnchorState>(&bytes, AUX_V1).is_err());
        let aux = AuxData::default();
        let bytes = encode_versioned(&aux, AUX_V1).unwrap();
        assert_eq!(decode_versioned::<AuxData>(&bytes, AUX_V1).unwrap(), aux);
        assert!(decode_versioned::<AuxData>(&bytes, STATE_V1).is_err());
    }

    #[test]
    fn state_rejects_unversioned_unknown_and_truncated_records() {
        let state = make_test_asm_state();
        let unversioned = encode_framed(state.state()).unwrap();
        let error = decode_versioned::<AnchorState>(&unversioned, STATE_V1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("reset and replay"));
        let mut bytes = encode_versioned(state.state(), STATE_V1).unwrap();
        bytes[7] = 2;
        assert!(decode_versioned::<AnchorState>(&bytes, STATE_V1).is_err());
        assert!(decode_versioned::<AnchorState>(STATE_V1, STATE_V1).is_err());
    }

    #[test]
    fn retained_logs_keep_individual_borsh_ssz_frames() {
        let logs = vec![
            AsmLogEntry::from_raw(vec![1, 2, 3]).unwrap(),
            AsmLogEntry::from_raw(vec![]).unwrap(),
        ];
        let bytes = encode_logs(&logs).unwrap();
        let frames: Vec<Vec<u8>> = borsh::from_slice(&bytes).unwrap();
        assert_eq!(
            frames,
            logs.iter().map(Encode::as_ssz_bytes).collect::<Vec<_>>()
        );
        assert_eq!(decode_logs(&bytes).unwrap(), logs);
        assert!(decode_logs(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_logs(&trailing).is_err());
    }

    #[test]
    fn retained_manifest_and_checkpoint_keep_borsh_ssz_frames() {
        let manifest = AsmManifest::new(1, Default::default(), Default::default(), vec![]).unwrap();
        let bytes = encode_framed(&manifest).unwrap();
        assert_eq!(
            borsh::from_slice::<Vec<u8>>(&bytes).unwrap(),
            manifest.as_ssz_bytes()
        );
        assert_eq!(decode_framed::<AsmManifest>(&bytes).unwrap(), manifest);
        let payload = create_test_checkpoint_payload(1);
        let bytes = encode_framed(&payload).unwrap();
        assert_eq!(
            borsh::from_slice::<Vec<u8>>(&bytes).unwrap(),
            payload.as_ssz_bytes()
        );
        assert_eq!(decode_framed::<CheckpointPayload>(&bytes).unwrap(), payload);
        assert!(decode_framed::<CheckpointPayload>(&bytes[..bytes.len() - 1]).is_err());
    }
}
