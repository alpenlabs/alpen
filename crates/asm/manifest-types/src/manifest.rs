use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use strata_identifiers::{Buf32, L1BlockId};
use tree_hash::{Sha256Hasher, TreeHash};

use crate::{
    Hash32,
    ssz_generated::ssz::{log::AsmLogEntry, manifest::AsmManifest},
};

impl AsmManifest {
    /// Creates a new ASM manifest.
    pub fn new(blkid: L1BlockId, wtxids_root: Buf32, logs: Vec<AsmLogEntry>) -> Self {
        Self {
            blkid,
            wtxids_root,
            logs: logs.into(),
        }
    }

    /// Returns the L1 block identifier.
    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }

    /// Returns the witness transaction ID merkle root.
    pub fn wtxids_root(&self) -> &Buf32 {
        &self.wtxids_root
    }

    /// Returns the log entries.
    pub fn logs(&self) -> &[AsmLogEntry] {
        &self.logs
    }

    /// Computes the hash of the manifest using SSZ tree hash.
    ///
    /// This uses SSZ to compute the root of the `AsmManifest` container, which
    /// enables creating Merkle inclusion proofs for individual fields (logs,
    /// `wtxids_root`, etc.) when needed.
    pub fn compute_hash(&self) -> Hash32 {
        let root = TreeHash::<Sha256Hasher>::tree_hash_root(self);
        Hash32::from(root.0)
    }
}

// Serde implementations delegate to fields
impl Serialize for AsmManifest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AsmManifest", 3)?;
        state.serialize_field("blkid", &self.blkid)?;
        state.serialize_field("wtxids_root", &self.wtxids_root)?;
        state.serialize_field("logs", &self.logs)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for AsmManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Blkid,
            WtxidsRoot,
            Logs,
        }

        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = AsmManifest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct AsmManifest")
            }

            fn visit_map<V>(self, mut map: V) -> Result<AsmManifest, V::Error>
            where
                V: serde::de::MapAccess<'de>,
            {
                let mut blkid = None;
                let mut wtxids_root = None;
                let mut logs = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Blkid => {
                            if blkid.is_some() {
                                return Err(serde::de::Error::duplicate_field("blkid"));
                            }
                            blkid = Some(map.next_value()?);
                        }
                        Field::WtxidsRoot => {
                            if wtxids_root.is_some() {
                                return Err(serde::de::Error::duplicate_field("wtxids_root"));
                            }
                            wtxids_root = Some(map.next_value()?);
                        }
                        Field::Logs => {
                            if logs.is_some() {
                                return Err(serde::de::Error::duplicate_field("logs"));
                            }
                            logs = Some(map.next_value()?);
                        }
                    }
                }
                let blkid = blkid.ok_or_else(|| serde::de::Error::missing_field("blkid"))?;
                let wtxids_root =
                    wtxids_root.ok_or_else(|| serde::de::Error::missing_field("wtxids_root"))?;
                let logs = logs.ok_or_else(|| serde::de::Error::missing_field("logs"))?;
                Ok(AsmManifest {
                    blkid,
                    wtxids_root,
                    logs,
                })
            }
        }

        deserializer.deserialize_struct("AsmManifest", &["blkid", "wtxids_root", "logs"], Visitor)
    }
}

// Borsh implementations are a shim over SSZ - just write/read SSZ bytes directly
impl BorshSerialize for AsmManifest {
    fn serialize<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        let ssz_bytes = self.as_ssz_bytes();
        writer.write_all(&ssz_bytes)
    }
}

impl BorshDeserialize for AsmManifest {
    fn deserialize_reader<R: borsh::io::Read>(reader: &mut R) -> borsh::io::Result<Self> {
        let mut ssz_bytes = Vec::new();
        reader.read_to_end(&mut ssz_bytes)?;
        AsmManifest::from_ssz_bytes(&ssz_bytes).map_err(|e| {
            borsh::io::Error::new(
                borsh::io::ErrorKind::InvalidData,
                format!("SSZ decode error: {:?}", e),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_identifiers::{Buf32, L1BlockId};
    use strata_test_utils_ssz::ssz_proptest;

    use super::AsmManifest;
    use crate::ssz_generated::ssz::log::AsmLogEntry;

    fn buf32_strategy() -> impl Strategy<Value = Buf32> {
        any::<[u8; 32]>().prop_map(Buf32::from)
    }

    fn l1_block_id_strategy() -> impl Strategy<Value = L1BlockId> {
        buf32_strategy().prop_map(L1BlockId::from)
    }

    fn asm_log_entry_strategy() -> impl Strategy<Value = AsmLogEntry> {
        prop::collection::vec(any::<u8>(), 0..256).prop_map(AsmLogEntry::from_raw)
    }

    fn asm_manifest_strategy() -> impl Strategy<Value = AsmManifest> {
        (
            l1_block_id_strategy(),
            buf32_strategy(),
            prop::collection::vec(asm_log_entry_strategy(), 0..10),
        )
            .prop_map(|(blkid, wtxids_root, logs)| AsmManifest::new(blkid, wtxids_root, logs))
    }

    mod asm_manifest {
        use super::*;

        ssz_proptest!(AsmManifest, asm_manifest_strategy());

        #[test]
        fn test_empty_logs() {
            let manifest = AsmManifest::new(
                L1BlockId::from(Buf32::from([0u8; 32])),
                Buf32::from([1u8; 32]),
                vec![],
            );
            let encoded = manifest.as_ssz_bytes();
            let decoded = AsmManifest::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(manifest.blkid(), decoded.blkid());
            assert_eq!(manifest.wtxids_root(), decoded.wtxids_root());
            assert_eq!(manifest.logs().len(), decoded.logs().len());
        }

        #[test]
        fn test_with_logs() {
            let logs = vec![
                AsmLogEntry::from_raw(vec![1, 2, 3]),
                AsmLogEntry::from_raw(vec![4, 5, 6]),
            ];
            let manifest = AsmManifest::new(
                L1BlockId::from(Buf32::from([0u8; 32])),
                Buf32::from([1u8; 32]),
                logs.clone(),
            );
            let encoded = manifest.as_ssz_bytes();
            let decoded = AsmManifest::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(manifest.logs().len(), decoded.logs().len());
            for (original, decoded_log) in manifest.logs().iter().zip(decoded.logs()) {
                assert_eq!(original.as_bytes(), decoded_log.as_bytes());
            }
        }

        #[test]
        fn test_compute_hash_deterministic() {
            let manifest = AsmManifest::new(
                L1BlockId::from(Buf32::from([0u8; 32])),
                Buf32::from([1u8; 32]),
                vec![AsmLogEntry::from_raw(vec![1, 2, 3])],
            );
            let hash1 = manifest.compute_hash();
            let hash2 = manifest.compute_hash();
            assert_eq!(hash1, hash2);
        }

        #[test]
        fn test_compute_hash_different_for_different_manifests() {
            let manifest1 = AsmManifest::new(
                L1BlockId::from(Buf32::from([0u8; 32])),
                Buf32::from([1u8; 32]),
                vec![AsmLogEntry::from_raw(vec![1, 2, 3])],
            );
            let manifest2 = AsmManifest::new(
                L1BlockId::from(Buf32::from([1u8; 32])),
                Buf32::from([1u8; 32]),
                vec![AsmLogEntry::from_raw(vec![1, 2, 3])],
            );
            let hash1 = manifest1.compute_hash();
            let hash2 = manifest2.compute_hash();
            assert_ne!(hash1, hash2);
        }
    }
}
