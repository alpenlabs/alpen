#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use ssz_types::FixedBytes;
use strata_crypto::hash;
use strata_identifiers::{L1BlockId, WtxidsRoot};
use tree_hash::{Sha256Hasher, TreeHash};

use crate::{
    Hash32,
    ssz_generated::ssz::{log::AsmLogEntry, manifest::AsmManifest},
};

impl AsmManifest {
    /// Creates a new ASM manifest.
    pub fn new(
        height: u64,
        blkid: L1BlockId,
        wtxids_root: WtxidsRoot,
        logs: Vec<AsmLogEntry>,
    ) -> Self {
        Self {
            height,
            blkid,
            wtxids_root,
            logs: logs.into(),
        }
    }

    /// Returns the L1 block height.
    pub fn height(&self) -> u64 {
        self.height
    }

    /// Returns the L1 block identifier.
    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }

    /// Returns the witness transaction ID merkle root.
    pub fn wtxids_root(&self) -> &WtxidsRoot {
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

// Borsh implementations are a shim over SSZ with length-prefixing to support nested structs
strata_identifiers::impl_borsh_via_ssz!(AsmManifest);

// Manual Arbitrary implementation for testing/benchmarking
#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for AsmManifest {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let height = u64::arbitrary(u)?;
        let blkid = L1BlockId::arbitrary(u)?;
        let wtxids_root = WtxidsRoot::arbitrary(u)?;

        // Generate a small number of logs for testing
        let num_logs = u.int_in_range(0..=10)?;
        let mut logs = Vec::with_capacity(num_logs);
        for _ in 0..num_logs {
            logs.push(AsmLogEntry::arbitrary(u)?);
        }

        Ok(AsmManifest::new(height, blkid, wtxids_root, logs))
    }
}

/// Computes a commitment hash over a sequence of ASM manifests.
///
/// This function concatenates the individual hashes of each manifest and
/// hashes the resulting byte sequence to produce a single commitment value.
pub fn compute_asm_manifests_hash(manifests: &[AsmManifest]) -> FixedBytes<32> {
    // Pre-allocate buffer for concatenated manifest hashes
    // Each manifest hash is 32 bytes
    let mut manifest_hashes_buf = Vec::with_capacity(manifests.len() * 32);

    // Concatenate individual manifest hashes
    for manifest in manifests {
        let manifest_hash = manifest.compute_hash();
        manifest_hashes_buf.extend_from_slice(&manifest_hash);
    }

    // Compute final commitment hash over the concatenated hashes
    hash::raw(&manifest_hashes_buf).into()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_identifiers::{
        Buf32, L1BlockId, WtxidsRoot,
        test_utils::{buf32_strategy, l1_block_id_strategy},
    };
    use strata_test_utils_ssz::ssz_proptest;

    use super::AsmManifest;
    use crate::ssz_generated::ssz::log::AsmLogEntry;

    fn wtxids_root_strategy() -> impl Strategy<Value = WtxidsRoot> {
        buf32_strategy().prop_map(WtxidsRoot::from)
    }

    fn asm_log_entry_strategy() -> impl Strategy<Value = AsmLogEntry> {
        prop::collection::vec(any::<u8>(), 0..256).prop_map(AsmLogEntry::from_raw)
    }

    fn asm_manifest_strategy() -> impl Strategy<Value = AsmManifest> {
        (
            any::<u64>(),
            l1_block_id_strategy(),
            wtxids_root_strategy(),
            prop::collection::vec(asm_log_entry_strategy(), 0..10),
        )
            .prop_map(|(height, blkid, wtxids_root, logs)| {
                AsmManifest::new(height, blkid, wtxids_root, logs)
            })
    }

    mod asm_manifest {
        use super::*;

        ssz_proptest!(AsmManifest, asm_manifest_strategy());

        #[test]
        fn test_empty_logs() {
            let manifest = AsmManifest::new(
                100,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([1u8; 32])),
                vec![],
            );
            let encoded = manifest.as_ssz_bytes();
            let decoded = AsmManifest::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(manifest.height(), decoded.height());
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
                200,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([1u8; 32])),
                logs.clone(),
            );
            let encoded = manifest.as_ssz_bytes();
            let decoded = AsmManifest::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(manifest.height(), decoded.height());
            assert_eq!(manifest.logs().len(), decoded.logs().len());
            for (original, decoded_log) in manifest.logs().iter().zip(decoded.logs()) {
                assert_eq!(original.as_bytes(), decoded_log.as_bytes());
            }
        }

        #[test]
        fn test_compute_hash_deterministic() {
            let manifest = AsmManifest::new(
                100,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([1u8; 32])),
                vec![AsmLogEntry::from_raw(vec![1, 2, 3])],
            );
            let hash1 = manifest.compute_hash();
            let hash2 = manifest.compute_hash();
            assert_eq!(hash1, hash2);
        }

        #[test]
        fn test_compute_hash_different_for_different_manifests() {
            let manifest1 = AsmManifest::new(
                100,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([1u8; 32])),
                vec![AsmLogEntry::from_raw(vec![1, 2, 3])],
            );
            let manifest2 = AsmManifest::new(
                100,
                L1BlockId::from(Buf32::from([1u8; 32])),
                WtxidsRoot::from(Buf32::from([1u8; 32])),
                vec![AsmLogEntry::from_raw(vec![1, 2, 3])],
            );
            let hash1 = manifest1.compute_hash();
            let hash2 = manifest2.compute_hash();
            assert_ne!(hash1, hash2);
        }
    }
}
