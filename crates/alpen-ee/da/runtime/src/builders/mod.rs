//! Host-side helpers for constructing the [`DaWitness`] the
//! [`verification`](crate::verification) module checks. Gated behind the
//! `builders` feature so guest/proof builds link only the verifier.
//!
//! - `inclusion` — the generic L1 byte-blob inclusion layer (walking the batch's L1 blocks, wtxid
//!   proofs, blob reassembly), execution-environment agnostic.
//! - `dedup` — the EVM bytecode-dedup layer: which bytecodes the blob omits, resolving their
//!   preimages, and the [`DedupWitnessResolver`] seam that produces the
//!   [`DedupWitness`](alpen_ee_da_types::DedupWitness).
//! - [`build_da_witness`] — the single entry point that orchestrates the two into a [`DaWitness`].

mod dedup;
mod error;
mod inclusion;

use alloy_primitives::B256;
use alpen_ee_common::L1DaBlockRef;
use alpen_ee_da_types::DaWitness;
use bitcoind_async_client::traits::Reader;
pub use dedup::{DaDedupResolver, DedupWitnessResolver};
pub use error::DaWitnessBuildError;
use strata_acct_types::Hash;

use self::inclusion::{collect_l1_inclusion_blocks, reassemble_da_blob_from_txs};

/// Builds the DA witness for a batch: the generic L1 tx-inclusion proofs plus the
/// supplementary dedup witness produced by `resolver`.
///
/// This is the single entry point the prover calls. It walks the batch's L1
/// blocks, reassembles the published blob (the "established as published" handoff
/// between the generic inclusion layer and the EVM dedup resolution), and returns
/// the [`DaWitness`].
pub async fn build_da_witness(
    da_refs: &[L1DaBlockRef],
    batch_block_hashes: &[Hash],
    btc: &(impl Reader + Sync),
    resolver: &(impl DedupWitnessResolver + Sync),
) -> Result<DaWitness, DaWitnessBuildError> {
    let (blocks, included_txs) = collect_l1_inclusion_blocks(da_refs, btc).await?;

    let blob = reassemble_da_blob_from_txs(&included_txs)?;

    let block_hashes: Vec<B256> = batch_block_hashes.iter().map(|h| B256::from(h.0)).collect();
    let dedup_da_witness = resolver.resolve_dedup_witness(&blob, &block_hashes).await?;

    Ok(DaWitness::new(blocks, dedup_da_witness))
}
