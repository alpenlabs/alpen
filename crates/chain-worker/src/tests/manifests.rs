//! An in-memory stand-in for the L1 manifest store.

use std::collections::HashMap;

use strata_asm_common::AsmManifest;
use strata_identifiers::L1BlockCommitment;

/// ASM manifests as the L1 store keeps them: every stored manifest by its L1
/// block, and the canonical block at each height.
///
/// A reorg leaves the replaced block's manifest stored by its block, so a
/// lookup by block can find a manifest that the lookup by height does not.
/// Manifests are keyed by height and block ID together because test fixtures
/// reuse block IDs across heights; real L1 block IDs are unique.
#[derive(Clone, Debug, Default)]
pub(super) struct MockManifests {
    by_block: HashMap<L1BlockCommitment, AsmManifest>,
    canonical: HashMap<u32, L1BlockCommitment>,
}

impl MockManifests {
    /// Stores `manifest` and makes its block the canonical one at its height.
    pub(super) fn insert_canonical(&mut self, manifest: AsmManifest) {
        let block = manifest_block(&manifest);
        self.canonical.insert(block.height(), block);
        self.by_block.insert(block, manifest);
    }

    /// Removes the manifest of the canonical block at `height`, leaving no
    /// canonical block there.
    pub(super) fn remove_canonical_at(&mut self, height: u32) -> Option<AsmManifest> {
        let block = self.canonical.remove(&height)?;
        self.by_block.remove(&block)
    }

    /// Returns the manifest of the canonical block at `height`.
    pub(super) fn canonical_at(&self, height: u32) -> Option<&AsmManifest> {
        self.canonical
            .get(&height)
            .and_then(|block| self.by_block.get(block))
    }

    /// Returns the stored manifest of `block`, canonical or not.
    pub(super) fn of_block(&self, block: &L1BlockCommitment) -> Option<&AsmManifest> {
        self.by_block.get(block)
    }
}

impl FromIterator<AsmManifest> for MockManifests {
    /// Stores each manifest as the canonical one at its height.
    fn from_iter<I: IntoIterator<Item = AsmManifest>>(manifests: I) -> Self {
        let mut store = Self::default();
        for manifest in manifests {
            store.insert_canonical(manifest);
        }
        store
    }
}

fn manifest_block(manifest: &AsmManifest) -> L1BlockCommitment {
    L1BlockCommitment::new(manifest.height(), *manifest.blkid())
}
