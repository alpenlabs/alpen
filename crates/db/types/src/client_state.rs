//! Client state database interface.

use strata_csm_types::{ClientState, ClientUpdateOutput};
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_primitives::prelude::*;

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Db for client state updates and checkpoints.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:client_state")
)]
pub trait ClientStateDatabase: Send + Sync + 'static {
    /// Writes a new consensus output for a given l1 block.
    fn put_client_update(
        &self,
        block: L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> DbResult<()>;

    /// Gets the output client state writes for some input index.
    fn get_client_update(&self, block: L1BlockCommitment) -> DbResult<Option<ClientUpdateOutput>>;

    /// Gets latest client state (the entry that corresponds to the highest l1 block).
    fn get_latest_client_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>>;

    /// Deletes a client update for a given l1 block.
    fn del_client_update(&self, block: L1BlockCommitment) -> DbResult<()>;

    /// Gets client updates starting from a given L1BlockCommitment up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    fn get_client_updates_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, ClientUpdateOutput)>>;
}
