//! Context for the batch builder task.

use std::sync::Arc;

use alpen_ee_common::{BatchStorage, ExecBlockStorage};
use alpen_ee_exec_chain::ExecChainHandle;
use strata_acct_types::Hash;
use tokio::sync::watch;

use super::{BatchBuilderConfig, BatchPolicy, BatchSealingPolicy, BlockDataProvider};

/// Context holding all dependencies for the batch builder task.
///
/// This struct contains everything the task needs except for the mutable state,
/// which is passed separately to allow for state recovery on restart.
pub(crate) struct BatchBuilderCtx<P, D, S, BS, ES>
where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage,
    ES: ExecBlockStorage,
{
    /// Genesis block hash, used as the starting point for the first batch.
    pub genesis_hash: Hash,
    /// Configuration for the batch builder.
    pub config: BatchBuilderConfig,
    /// Receiver for canonical tip updates from ExecChain.
    pub preconf_rx: watch::Receiver<Hash>,
    /// Provider for fetching block data (e.g., DA size).
    pub block_data_provider: Arc<D>,
    /// Policy for determining when to seal a batch.
    pub sealing_policy: S,
    /// Storage for exec blocks.
    pub block_storage: Arc<ES>,
    /// Storage for batches.
    pub batch_storage: Arc<BS>,
    /// Handle to query canonical chain status.
    pub exec_chain: Arc<ExecChainHandle>,
    /// Marker for the policy type.
    pub _policy: std::marker::PhantomData<P>,
}
