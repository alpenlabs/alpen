//! Canonical [`LedgerRefs`] construction from a batch's DA refs.
//!
//! Used by both the OL submitter (when building the on-chain SAU
//! transaction) and the prover (when assembling pub-params for the
//! update proof). The two paths MUST produce byte-identical
//! [`LedgerRefs`] — otherwise the verifier's claim reconstruction won't
//! match the prover-committed pub-params SSZ and Groth16 verification
//! will fail.

use std::collections::HashMap;

use futures::future::try_join_all;
use strata_acct_types::Hash;
use strata_identifiers::L1Height;
use strata_snark_acct_types::{AccumulatorClaim, LedgerRefs};

use crate::{
    traits::ol_client::{OLClientError, SequencerOLClient},
    types::batch::L1DaBlockRef,
};

/// Errors produced when building [`LedgerRefs`] from a batch's DA refs.
#[derive(Debug, thiserror::Error)]
pub enum LedgerRefsError {
    /// Failed to fetch the canonical L1 header commitment for a given height.
    #[error("get_asm_manifest_commitment({height}): {source}")]
    FetchCommitment {
        height: L1Height,
        #[source]
        source: OLClientError,
    },
}

/// Builds canonical [`LedgerRefs`] from `da_refs`.
///
/// Resolves each L1 height to its canonical L1 header commitment via
/// `ol_client`, uses the raw L1 height as the MMR leaf index (the OL
/// ASM-manifests MMR is height-indexed, prefilled at genesis with
/// zero-hash leaves), then sorts and dedups by index (multiple DA txns
/// may land in one L1 block).
///
/// The `?Sized` relaxation on the `impl SequencerOLClient` bound is
/// required so that callers may pass a `&dyn SequencerOLClient`; the
/// implicit `Sized` bound on `impl Trait` would otherwise reject it.
pub async fn build_ledger_refs_from_da(
    da_refs: &[L1DaBlockRef],
    ol_client: &(impl SequencerOLClient + ?Sized),
) -> Result<LedgerRefs, LedgerRefsError> {
    let asm_manifest_commitments =
        fetch_asm_manifest_commitments_by_height(da_refs, ol_client).await?;
    Ok(build_ledger_refs(da_refs, &asm_manifest_commitments))
}

async fn fetch_asm_manifest_commitments_by_height(
    da_refs: &[L1DaBlockRef],
    ol_client: &(impl SequencerOLClient + ?Sized),
) -> Result<HashMap<L1Height, Hash>, LedgerRefsError> {
    let mut heights: Vec<L1Height> = da_refs.iter().map(|da_ref| da_ref.block.height()).collect();
    heights.sort_unstable();
    heights.dedup();

    let pairs = try_join_all(heights.into_iter().map(|height| async move {
        let hash = ol_client
            .get_asm_manifest_commitment(height)
            .await
            .map_err(|source| LedgerRefsError::FetchCommitment { height, source })?;
        Ok::<_, LedgerRefsError>((height, hash))
    }))
    .await?;

    Ok(pairs.into_iter().collect())
}

fn build_ledger_refs(
    da_refs: &[L1DaBlockRef],
    asm_manifest_commitments: &HashMap<L1Height, Hash>,
) -> LedgerRefs {
    let mut asm_manifest_refs: Vec<AccumulatorClaim> = da_refs
        .iter()
        .map(|da_ref| {
            let height = da_ref.block.height();
            // `fetch_asm_manifest_commitments_by_height` populates an entry for
            // every height present in `da_refs`, so a miss here is a bug, not
            // a transient error — leave it as an `expect` to surface that.
            let hash = *asm_manifest_commitments
                .get(&height)
                .expect("commitment map covers every DA-ref height");
            AccumulatorClaim::new(height as u64, *hash.as_ref())
        })
        .collect();

    asm_manifest_refs.sort_by_key(|c| c.idx());
    asm_manifest_refs.dedup_by_key(|c| c.idx());

    LedgerRefs::new(asm_manifest_refs)
}
