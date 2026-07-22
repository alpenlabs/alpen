//! L1 data database interface.

// TODO(trey): wrap AsmManifest type in versionable container

use strata_asm_common::AsmManifest;
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_primitives::l1::L1BlockId;
use strata_primitives::L1Height;

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Database interface to control our view of L1 data.
/// Operations are NOT VALIDATED at this level.
/// Ensure all operations are done through `L1BlockManager`
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:l1")
)]
pub trait L1Database: Send + Sync + 'static {
    /// Stores an ASM manifest for a given L1 block.
    /// Returns error if provided out-of-order.
    fn put_block_data(&self, manifest: AsmManifest) -> DbResult<()>;

    /// Set a specific height, blockid in canonical chain records.
    fn set_canonical_chain_entry(&self, height: L1Height, blockid: L1BlockId) -> DbResult<()>;

    /// remove canonical chain records in given range (inclusive)
    fn remove_canonical_chain_entries(
        &self,
        start_height: L1Height,
        end_height: L1Height,
    ) -> DbResult<()>;

    /// Prune earliest blocks till height
    fn prune_to_height(&self, height: L1Height) -> DbResult<()>;

    // TODO(STR-2653): DA scraping storage

    // Gets current chain tip height, blockid
    fn get_canonical_chain_tip(&self) -> DbResult<Option<(L1Height, L1BlockId)>>;

    /// Gets the ASM manifest for a blockid.
    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<AsmManifest>>;

    /// Gets the blockid at height for the current chain.
    fn get_canonical_blockid_at_height(&self, height: L1Height) -> DbResult<Option<L1BlockId>>;

    // TODO(STR-2653): This should not exist in database level and should be handled by downstream
    // manager.
    /// Returns a half-open interval of block hashes, if we have all of them
    /// present.  Otherwise, returns error.
    fn get_canonical_blockid_range(
        &self,
        start_idx: L1Height,
        end_idx: L1Height,
    ) -> DbResult<Vec<L1BlockId>>;

    // TODO(STR-2653): DA queries
}
