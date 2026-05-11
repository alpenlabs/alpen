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
    #[error("get_l1_header_commitment({height}): {source}")]
    FetchCommitment {
        height: L1Height,
        #[source]
        source: OLClientError,
    },

    /// An L1 height in the DA refs is before the configured MMR start offset.
    #[error("L1 height {height} is before MMR start offset {mmr_offset}")]
    OffsetUnderflow { height: L1Height, mmr_offset: u64 },
}

/// Builds canonical [`LedgerRefs`] from `da_refs`.
///
/// Resolves each L1 height to its canonical L1 header commitment via
/// `ol_client`, maps it to its MMR leaf index using `genesis_l1_height + 1`
/// as the offset, then sorts and dedups by index (multiple DA txns may
/// land in one L1 block).
pub async fn build_ledger_refs_from_da<C>(
    da_refs: &[L1DaBlockRef],
    ol_client: &C,
    genesis_l1_height: L1Height,
) -> Result<LedgerRefs, LedgerRefsError>
where
    C: SequencerOLClient + Send + Sync + ?Sized,
{
    let l1_header_commitments = fetch_l1_header_commitments_by_height(da_refs, ol_client).await?;
    build_ledger_refs(da_refs, &l1_header_commitments, genesis_l1_height)
}

async fn fetch_l1_header_commitments_by_height<C>(
    da_refs: &[L1DaBlockRef],
    ol_client: &C,
) -> Result<HashMap<L1Height, Hash>, LedgerRefsError>
where
    C: SequencerOLClient + Send + Sync + ?Sized,
{
    let mut heights: Vec<L1Height> = da_refs.iter().map(|da_ref| da_ref.block.height()).collect();
    heights.sort_unstable();
    heights.dedup();

    let pairs = try_join_all(heights.into_iter().map(|height| async move {
        let hash = ol_client
            .get_l1_header_commitment(height)
            .await
            .map_err(|source| LedgerRefsError::FetchCommitment { height, source })?;
        Ok::<_, LedgerRefsError>((height, hash))
    }))
    .await?;

    Ok(pairs.into_iter().collect())
}

fn build_ledger_refs(
    da_refs: &[L1DaBlockRef],
    l1_header_commitments: &HashMap<L1Height, Hash>,
    genesis_l1_height: L1Height,
) -> Result<LedgerRefs, LedgerRefsError> {
    let mmr_offset = genesis_l1_height as u64 + 1;
    let mut l1_header_refs: Vec<AccumulatorClaim> = da_refs
        .iter()
        .map(|da_ref| {
            let height = da_ref.block.height();
            // `fetch_l1_header_commitments_by_height` populates an entry for
            // every height present in `da_refs`, so a miss here is a bug, not
            // a transient error — leave it as an `expect` to surface that.
            let hash = *l1_header_commitments
                .get(&height)
                .expect("commitment map covers every DA-ref height");
            let mmr_idx = (height as u64)
                .checked_sub(mmr_offset)
                .ok_or(LedgerRefsError::OffsetUnderflow { height, mmr_offset })?;
            Ok(AccumulatorClaim::new(mmr_idx, *hash.as_ref()))
        })
        .collect::<Result<Vec<_>, LedgerRefsError>>()?;

    l1_header_refs.sort_by_key(|c| c.idx());
    l1_header_refs.dedup_by_key(|c| c.idx());

    Ok(LedgerRefs::new(l1_header_refs))
}
