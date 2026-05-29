//! Canonical [`LedgerRefs`] construction from a batch's DA refs.
//!
//! Used by both the OL submitter (when building the on-chain SAU
//! transaction) and the prover (when assembling pub-params for the
//! update proof). The two paths MUST produce byte-identical
//! [`LedgerRefs`] — otherwise the verifier's claim reconstruction won't
//! match the prover-committed pub-params SSZ and Groth16 verification
//! will fail.

use strata_acct_types::{l1_block_record_leaf_hash, AccumulatorClaim};
use strata_snark_acct_types::LedgerRefs;

use crate::types::batch::L1DaBlockRef;

/// Builds canonical [`LedgerRefs`] from `da_refs`.
///
/// Uses the raw L1 height as the MMR leaf index, and commits each entry to its
/// `{block_hash, wtxids_root}` via [`l1_block_record_leaf_hash`]; then sorts and
/// dedups by index because multiple DA txns from the same batch may land in one
/// L1 block.
pub fn build_ledger_refs_from_da(da_refs: &[L1DaBlockRef]) -> LedgerRefs {
    let mut l1_block_refs: Vec<AccumulatorClaim> = da_refs
        .iter()
        .map(|da_ref| {
            let height = da_ref.block.height();
            let entry_hash = l1_block_record_leaf_hash(
                da_ref.block.blkid().as_ref(),
                da_ref.block.wtxids_root().as_ref(),
            );
            AccumulatorClaim::new(height as u64, entry_hash)
        })
        .collect();

    l1_block_refs.sort_by_key(|c| c.idx());
    l1_block_refs.dedup_by_key(|c| c.idx());

    LedgerRefs::new(l1_block_refs)
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, WtxidsRoot};

    use super::*;
    use crate::types::batch::L1DaBlockInfo;

    fn da_ref(height: u32, block_byte: u8, wtxids_byte: u8) -> L1DaBlockRef {
        let block_hash = [block_byte; 32];
        let wtxids_root = [wtxids_byte; 32];
        let block = L1DaBlockInfo::new(
            L1BlockCommitment::new(height, L1BlockId::from(Buf32::from(block_hash))),
            WtxidsRoot::from(Buf32::from(wtxids_root)),
        );
        L1DaBlockRef::new(block, Vec::new())
    }

    #[test]
    fn build_ledger_refs_commits_l1_block_ref_hash() {
        let refs = [da_ref(7, 1, 2)];

        let ledger_refs = build_ledger_refs_from_da(&refs);
        let claims = ledger_refs.l1_block_refs();

        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].idx(), 7);
        assert_eq!(
            claims[0].entry_hash().as_ref(),
            l1_block_record_leaf_hash(&[1; 32], &[2; 32]).as_slice()
        );
    }

    #[test]
    fn build_ledger_refs_sorts_and_dedups_by_height() {
        let refs = [da_ref(9, 9, 9), da_ref(4, 4, 4), da_ref(9, 9, 9)];

        let ledger_refs = build_ledger_refs_from_da(&refs);
        let indices: Vec<_> = ledger_refs
            .l1_block_refs()
            .iter()
            .map(AccumulatorClaim::idx)
            .collect();

        assert_eq!(indices, [4, 9]);
    }
}
