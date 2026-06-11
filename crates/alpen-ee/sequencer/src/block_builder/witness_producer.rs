//! Per-block proof-witness production hook for the block builder.

use async_trait::async_trait;
use strata_acct_types::Hash;

/// Produces and persists the depth-0 proof witness for a just-produced block.
///
/// The block builder calls this synchronously after submitting the payload to
/// the engine and **before** persisting/advancing the block: a block is not
/// accepted unless its witness has been persisted. Because it runs while the
/// block is still at tip, the witness's multiproofs are always at depth 0–1,
/// and the downstream proving pipeline never waits on a lagging out-of-band
/// producer.
#[async_trait]
pub trait BlockWitnessProducer: Send + Sync {
    /// Compute and persist the proof witness for the block identified by
    /// `block_hash`. Returning an error fails block production (which retries),
    /// so the block is never accepted without its witness.
    async fn produce_block_witness(&self, block_hash: Hash) -> eyre::Result<()>;
}
