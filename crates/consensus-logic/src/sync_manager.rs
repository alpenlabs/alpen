//! High level sync manager which controls core sync tasks and manages sync
//! status.  Exposes handles to interact with fork choice manager and CSM
//! executor and other core sync pipeline tasks.

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::{bail, Context};
use bitcoind_async_client::{error::ClientError, traits::Reader, Client};
use strata_asm_params::AsmParams;
use strata_asm_spec::StrataAsmSpec;
#[cfg(feature = "debug-asm")]
use strata_asm_spec_debug::DebugAsmSpec;
use strata_asm_worker::{AsmState as WorkerAsmState, AsmWorkerHandle, AsmWorkerStatus};
use strata_btc_types::{BlockHashExt, L1BlockIdBitcoinExt};
use strata_csm_worker::{CsmWorkerService, CsmWorkerState, CsmWorkerStatus};
use strata_node_context::NodeContext;
use strata_ol_state_types::MMR_SENTINEL_DUMMY_LEAF_HASH;
use strata_primitives::prelude::{L1BlockCommitment, L1BlockId, L1Height};
use strata_service::{ServiceBuilder, ServiceMonitor, SyncAsyncInput};
use strata_state::asm_state::AsmState as StorageAsmState;
use strata_status::StatusChannel;
use strata_storage::{MmrId, MmrIndexHandle, NodeStorage};
use strata_tasks::TaskExecutor;
use tokio::{runtime::Handle, time::sleep};
use tracing::{debug, info, warn};

use crate::{asm_worker_context::AsmWorkerCtx, csm_worker_context::CsmWorkerContextImpl};

#[cfg(not(test))]
const STARTUP_RECONCILE_RPC_RETRY_DELAY: Duration = Duration::from_secs(5);
#[cfg(test)]
const STARTUP_RECONCILE_RPC_RETRY_DELAY: Duration = Duration::from_millis(1);

/// Holds startup inputs that seed CSM before it subscribes to live ASM updates.
struct CsmListenerStartup<'a> {
    /// Sends CSM worker status updates to the node status channel.
    status_channel: Arc<StatusChannel>,
    /// Supplies live ASM status updates after the startup seed.
    asm_monitor: &'a ServiceMonitor<AsmWorkerStatus>,
    /// Reads Bitcoin blocks while CSM catches up from ASM commitments.
    bitcoin_client: Arc<Client>,
    /// Identifies the reconciled ASM block CSM should process first at startup.
    startup_asm_block: Option<L1BlockCommitment>,
}

/// Spawns the CSM listener using node context and the reconciled ASM startup block.
///
/// The listener receives the reconciled startup block before live ASM updates so
/// CSM can replay any canonical gap from its last persisted client-state anchor.
pub fn spawn_csm_listener_with_ctx(
    nodectx: &NodeContext,
    asm_monitor: &ServiceMonitor<AsmWorkerStatus>,
    startup_asm_block: Option<L1BlockCommitment>,
) -> anyhow::Result<ServiceMonitor<CsmWorkerStatus>> {
    let startup = CsmListenerStartup {
        status_channel: nodectx.status_channel().clone(),
        asm_monitor,
        bitcoin_client: nodectx.bitcoin_client().clone(),
        startup_asm_block,
    };

    spawn_csm_listener(
        nodectx.executor(),
        nodectx.asm_params().clone(),
        nodectx.config().btcio.l1_reorg_safe_depth,
        nodectx.storage().clone(),
        startup,
    )
}

fn spawn_csm_listener(
    executor: &TaskExecutor,
    asm_params: Arc<AsmParams>,
    l1_reorg_safe_depth: u32,
    storage: Arc<NodeStorage>,
    startup: CsmListenerStartup<'_>,
) -> anyhow::Result<ServiceMonitor<CsmWorkerStatus>> {
    // Create CSM worker state.
    let ctx = CsmWorkerContextImpl::new(
        executor.handle().clone(),
        startup.bitcoin_client,
        asm_params,
        l1_reorg_safe_depth,
        storage.clone(),
        startup.status_channel,
    );
    let csm_state = CsmWorkerState::init_from_context(ctx)?;

    let initial_updates = initial_asm_status_updates(storage.as_ref(), startup.startup_asm_block)?;

    // Create an input that listens to ASM status updates with the startup seed prepended.
    let async_input = startup
        .asm_monitor
        .create_listener_input_with(executor, initial_updates);
    // Wrap in SyncAsyncInput adapter since CSM worker is a sync service.
    let csm_input = SyncAsyncInput::new(async_input, executor.handle().clone());

    // Launch the CSM worker service (which acts as a listener to ASM worker).
    let csm_monitor = ServiceBuilder::<CsmWorkerService<CsmWorkerContextImpl>, _>::new()
        .with_state(csm_state)
        .with_input(csm_input)
        .launch_sync("csm_worker", executor)?;

    Ok(csm_monitor)
}

/// Reconciles stored L1 canonical state during startup and submits the target to ASM.
///
/// Returns the reconciled canonical ASM block CSM should use as its startup
/// seed. `None` means startup had no stored L1 canonical target to submit.
pub async fn reconcile_l1_storage_and_submit_to_asm(
    storage: Arc<NodeStorage>,
    bitcoin_client: Arc<Client>,
    asm_handle: &AsmWorkerHandle,
    genesis_l1_height: L1Height,
    l1_reorg_safe_depth: u32,
) -> anyhow::Result<Option<L1BlockCommitment>> {
    if !should_reconcile_l1_storage_at_startup(storage.as_ref(), genesis_l1_height).await? {
        return Ok(None);
    }

    let bitcoind_tip_height = retry_bitcoin_startup_rpc("fetching Bitcoin block count", || {
        let bitcoin_client = bitcoin_client.clone();
        async move {
            let height = bitcoin_client
                .get_block_count()
                .await
                .context("fetching Bitcoin block count during startup reconciliation")?;
            L1Height::try_from(height).context("Bitcoin block count exceeds L1 height range")
        }
    })
    .await?;

    let Some(target) = reconcile_l1_canonical_storage(
        storage.as_ref(),
        genesis_l1_height,
        l1_reorg_safe_depth,
        bitcoind_tip_height,
        |height| {
            let bitcoin_client = bitcoin_client.clone();
            retry_bitcoin_startup_rpc("fetching Bitcoin block hash", move || {
                let bitcoin_client = bitcoin_client.clone();
                async move {
                    let block_hash = bitcoin_client
                        .get_block_hash(height as u64)
                        .await
                        .with_context(|| {
                            format!("fetching Bitcoin block hash at startup height {height}")
                        })?;
                    Ok(block_hash.to_l1_block_id())
                }
            })
        },
    )
    .await?
    else {
        return Ok(None);
    };

    let processed_blocks = asm_handle
        .submit_block_async(target.blkid().to_block_hash())
        .await
        .with_context(|| format!("submitting startup L1 tip {target} to ASM"))?
        .len();

    info!(
        %target,
        processed_blocks,
        "submitted stored L1 canonical tip to ASM during startup"
    );
    Ok(Some(target))
}

/// Returns `true` when startup reconciliation needs Bitcoin RPC data.
///
/// Empty L1 storage and pre-genesis stored tips do not require reconciliation,
/// so startup avoids Bitcoin RPCs and lets the BTCIO reader perform its normal
/// retry loop.
async fn should_reconcile_l1_storage_at_startup(
    storage: &NodeStorage,
    genesis_l1_height: L1Height,
) -> anyhow::Result<bool> {
    let Some((stored_tip_height, _)) = storage.l1().get_canonical_chain_tip_async().await? else {
        debug!("no stored L1 canonical tip found during startup reconciliation");
        return Ok(false);
    };

    if stored_tip_height < genesis_l1_height {
        debug!(
            stored_tip_height,
            genesis_l1_height,
            "stored L1 canonical tip is before ASM genesis; skipping startup ASM submission"
        );
        return Ok(false);
    }

    Ok(true)
}

/// Reconciles stored L1 canonical state against bitcoind.
///
/// The returned block is the canonical ASM startup seed CSM should use. `None`
/// means there was no stored L1 target that should be submitted.
async fn reconcile_l1_canonical_storage<F, Fut>(
    storage: &NodeStorage,
    genesis_l1_height: L1Height,
    l1_reorg_safe_depth: u32,
    bitcoind_tip_height: L1Height,
    mut canonical_blockid_at_height: F,
) -> anyhow::Result<Option<L1BlockCommitment>>
where
    F: FnMut(L1Height) -> Fut,
    Fut: Future<Output = anyhow::Result<L1BlockId>>,
{
    let Some((stored_tip_height, stored_tip_blkid)) =
        storage.l1().get_canonical_chain_tip_async().await?
    else {
        debug!("no stored L1 canonical tip found during startup reconciliation");
        return Ok(None);
    };

    if stored_tip_height < genesis_l1_height {
        debug!(
            stored_tip_height,
            genesis_l1_height,
            "stored L1 canonical tip is before ASM genesis; skipping startup ASM submission"
        );
        return Ok(None);
    }

    let max_rewind_depth = l1_reorg_safe_depth.max(1).saturating_sub(1);
    let safe_rewind_floor = stored_tip_height
        .saturating_sub(max_rewind_depth)
        .max(genesis_l1_height);
    let bitcoind_is_behind = bitcoind_tip_height < stored_tip_height;
    if bitcoind_is_behind && bitcoind_tip_height < safe_rewind_floor {
        bail!(
            "bitcoind tip {bitcoind_tip_height} is too far behind stored L1 canonical tip \
             {stored_tip_height}; waiting for bitcoind to reach at least {safe_rewind_floor}"
        );
    }

    let search_tip = stored_tip_height.min(bitcoind_tip_height);
    let floor = search_tip.min(safe_rewind_floor);
    let mut observed_divergence = false;

    for height in (floor..=search_tip).rev() {
        let stored_blkid = if height == stored_tip_height {
            stored_tip_blkid
        } else {
            storage
                .l1()
                .get_canonical_blockid_at_height_async(height)
                .await?
                .with_context(|| format!("stored L1 canonical chain is missing height {height}"))?
        };
        let bitcoin_blkid = canonical_blockid_at_height(height).await?;

        if stored_blkid != bitcoin_blkid {
            observed_divergence = true;
            continue;
        }

        if height < stored_tip_height {
            if bitcoind_is_behind && !observed_divergence {
                info!(
                    stored_tip_height,
                    bitcoind_tip_height,
                    matched_height = height,
                    "bitcoind is behind stored L1 canonical tip; rewinding stored suffix for replay"
                );
            } else {
                warn!(
                    stored_tip_height,
                    pivot_height = height,
                    "stored L1 canonical suffix diverged from bitcoind; rewinding before startup"
                );
            }

            storage
                .l1()
                .revert_canonical_chain_async(height)
                .await
                .with_context(|| {
                    format!("rewinding stored L1 canonical chain to height {height}")
                })?;
        }

        return Ok(Some(L1BlockCommitment::new(height, stored_blkid)));
    }

    bail!(
        "stored L1 canonical tip {stored_tip_height} has no bitcoind pivot in startup lookback \
         [{floor}, {search_tip}]"
    )
}

/// Builds the initial ASM status seed for CSM startup.
///
/// When startup reconciled L1 storage, the seed is taken from that canonical
/// target instead of the lexicographically latest ASM row, which may be an
/// orphan left behind by an offline reorg.
fn initial_asm_status_updates(
    storage: &NodeStorage,
    startup_asm_block: Option<L1BlockCommitment>,
) -> anyhow::Result<Vec<AsmWorkerStatus>> {
    let state = match startup_asm_block {
        Some(block) => storage
            .asm()
            .get_state_blocking(block)?
            .map(|state| (block, state))
            .with_context(|| format!("startup ASM state {block} missing after reconciliation"))?,
        None => {
            let Some((block, state)) = storage.asm().fetch_most_recent_state_blocking()? else {
                return Ok(Vec::new());
            };
            match storage
                .l1()
                .get_canonical_blockid_at_height(block.height())?
            {
                Some(blockid) if blockid == *block.blkid() => (block, state),
                _ => {
                    warn!(
                        %block,
                        "skipping startup ASM seed because it is not on the stored L1 canonical chain"
                    );
                    return Ok(Vec::new());
                }
            }
        }
    };

    Ok(vec![AsmWorkerStatus {
        is_initialized: true,
        cur_block: Some(state.0),
        cur_state: Some(storage_to_worker_state(state.1)),
    }])
}

/// Returns `true` when an error wraps a retryable Bitcoin RPC failure.
fn is_retryable_bitcoin_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<ClientError>()
            .is_some_and(|err| err.is_retriable() || is_bitcoind_warmup_error(err))
    })
}

/// Returns `true` when bitcoind is reachable but still in RPC warmup.
fn is_bitcoind_warmup_error(err: &ClientError) -> bool {
    matches!(err, ClientError::Server(-28, _))
}

/// Runs a Bitcoin RPC operation with retries for transient failures.
///
/// Non-retryable errors fail immediately. Retryable errors are retried with
/// [`STARTUP_RECONCILE_RPC_RETRY_DELAY`] between attempts so startup waits for
/// bitcoind instead of aborting before the BTCIO reader is launched.
async fn retry_bitcoin_startup_rpc<T, F, Fut>(
    operation: &'static str,
    mut f: F,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 1;
    loop {
        match f().await {
            Ok(value) => return Ok(value),
            Err(err) if is_retryable_bitcoin_error(&err) => {
                warn!(
                    operation,
                    attempt,
                    %err,
                    "retryable Bitcoin RPC error during startup reconciliation"
                );
                sleep(STARTUP_RECONCILE_RPC_RETRY_DELAY).await;
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

pub fn spawn_asm_worker_with_ctx(nodectx: &NodeContext) -> anyhow::Result<AsmWorkerHandle> {
    spawn_asm_worker(
        nodectx.executor(),
        nodectx.executor().handle().clone(),
        nodectx.storage().clone(),
        nodectx.asm_params().clone(),
        nodectx.bitcoin_client().clone(),
    )
}

pub fn spawn_asm_worker(
    executor: &TaskExecutor,
    handle: Handle,
    storage: Arc<NodeStorage>,
    asm_params: Arc<AsmParams>,
    bitcoin_client: Arc<Client>,
) -> anyhow::Result<AsmWorkerHandle> {
    // This feels weird to pass both L1BlockManager and Bitcoin client, but ASM consumes raw bitcoin
    // blocks while following canonical chain (and "canonicity" of l1 chain is imposed by the l1
    // block manager).
    let mmr_handle = storage.mmr_index().get_handle(MmrId::Asm);

    // Prefill the ASM manifest MMR with dummy-hash leaves up to and including
    // the genesis L1 height, so that the manifest for height `h` lands at MMR
    // index `h`. This mirrors the in-memory OL state initialization.
    let genesis_l1_height = asm_params.anchor.block.height() as u64;
    prefill_asm_mmr(&mmr_handle, genesis_l1_height + 1)?;

    let context = AsmWorkerCtx::new(
        handle.clone(),
        bitcoin_client,
        storage.l1().clone(),
        storage.asm().clone(),
        mmr_handle,
    );

    // Construct the ASM spec based on the enabled feature.
    #[cfg(not(feature = "debug-asm"))]
    let asm_spec = StrataAsmSpec;
    #[cfg(feature = "debug-asm")]
    let asm_spec = DebugAsmSpec::new(StrataAsmSpec);

    // Use the new builder API to launch the worker and get a handle.
    let handle = strata_asm_worker::AsmWorkerBuilder::new()
        .with_context(context)
        .with_params((*asm_params).clone())
        .with_asm_spec(asm_spec)
        .launch(executor)?;

    Ok(handle)
}

fn storage_to_worker_state(state: StorageAsmState) -> WorkerAsmState {
    WorkerAsmState::new(state.state().clone(), state.logs().clone())
}

/// Prefills the ASM manifest MMR with sentinel leaves until it has at least
/// `target_count` entries.
///
/// This is idempotent: a no-op when the MMR already has at least
/// `target_count` entries. It is used to align DB-side MMR leaf indices with
/// L1 block heights, mirroring the in-memory OL state initialization.
fn prefill_asm_mmr(handle: &MmrIndexHandle, target_count: u64) -> anyhow::Result<()> {
    let current = handle.get_num_leaves_blocking()?;
    if current >= target_count {
        return Ok(());
    }

    for _ in current..target_count {
        handle.append_leaf_blocking(MMR_SENTINEL_DUMMY_LEAF_HASH)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::Buf32;
    use strata_storage::create_node_storage;
    use tokio::runtime::Handle;

    use super::*;

    /// Creates isolated node storage backed by a temporary sled database.
    fn test_storage() -> NodeStorage {
        create_node_storage(get_test_sled_backend(), Handle::current())
            .expect("create test storage")
    }

    /// Builds a deterministic L1 block id for test fixtures.
    fn blockid(byte: u8) -> L1BlockId {
        L1BlockId::from(Buf32::from([byte; 32]))
    }

    /// Seeds contiguous stored L1 canonical entries for tests.
    async fn seed_l1_chain(storage: &NodeStorage, start: L1Height, end: L1Height) {
        for height in start..=end {
            storage
                .l1()
                .extend_canonical_chain_async(&blockid(height as u8), height)
                .await
                .expect("extend L1 canonical chain");
        }
    }

    /// Verifies startup Bitcoin RPCs keep retrying transient failures until success.
    #[tokio::test]
    async fn startup_rpc_retry_waits_for_retryable_errors() {
        let attempts = AtomicUsize::new(0);

        let value = retry_bitcoin_startup_rpc("test startup rpc", || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                if attempt < 3 {
                    Err(anyhow::Error::from(ClientError::Connection(
                        "connection refused".to_string(),
                    )))
                } else {
                    Ok(42)
                }
            }
        })
        .await
        .expect("retryable errors should eventually succeed");

        assert_eq!(value, 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    /// Verifies startup Bitcoin RPCs retry bitcoind warmup responses.
    #[tokio::test]
    async fn startup_rpc_retry_waits_for_bitcoind_warmup() {
        let attempts = AtomicUsize::new(0);

        let value = retry_bitcoin_startup_rpc("test startup rpc", || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                if attempt < 3 {
                    Err(anyhow::Error::from(ClientError::Server(
                        -28,
                        "Loading block index...".to_string(),
                    )))
                } else {
                    Ok(42)
                }
            }
        })
        .await
        .expect("bitcoind warmup should eventually succeed");

        assert_eq!(value, 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    /// Verifies wrapped retryable startup errors are still classified.
    #[test]
    fn startup_rpc_retry_classifies_context_wrapped_errors() {
        let connection_err =
            anyhow::Error::from(ClientError::Connection("connection refused".to_string()))
                .context("fetching Bitcoin block count during startup reconciliation");
        let warmup_err = anyhow::Error::from(ClientError::Server(
            -28,
            "Loading block index...".to_string(),
        ))
        .context("fetching Bitcoin block hash during startup reconciliation");

        assert!(is_retryable_bitcoin_error(&connection_err));
        assert!(is_retryable_bitcoin_error(&warmup_err));
    }

    /// Verifies startup Bitcoin RPCs fail immediately for non-retryable errors.
    #[tokio::test]
    async fn startup_rpc_retry_fails_fast_on_non_retryable_errors() {
        let attempts = AtomicUsize::new(0);

        let err = retry_bitcoin_startup_rpc("test startup rpc", || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {
                Err::<(), _>(anyhow::Error::from(ClientError::Server(
                    -8,
                    "block height out of range".to_string(),
                )))
            }
        })
        .await
        .expect_err("non-retryable errors should fail immediately");

        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(
            err.to_string().contains("block height out of range"),
            "unexpected error: {err:#}"
        );
    }

    /// Verifies fresh storage does not require Bitcoin RPCs during startup reconciliation.
    #[tokio::test]
    async fn startup_reconciliation_does_not_need_rpc_without_stored_l1_tip() {
        let storage = test_storage();

        let should_reconcile = should_reconcile_l1_storage_at_startup(&storage, 10)
            .await
            .expect("check reconciliation need");

        assert!(!should_reconcile);
    }

    /// Verifies pre-genesis stored L1 tips do not require startup Bitcoin RPCs.
    #[tokio::test]
    async fn startup_reconciliation_does_not_need_rpc_before_asm_genesis() {
        let storage = test_storage();
        seed_l1_chain(&storage, 3, 5).await;

        let should_reconcile = should_reconcile_l1_storage_at_startup(&storage, 10)
            .await
            .expect("check reconciliation need");

        assert!(!should_reconcile);
    }

    /// Verifies post-genesis stored L1 tips require startup Bitcoin RPCs.
    #[tokio::test]
    async fn startup_reconciliation_needs_rpc_for_stored_l1_tip_at_asm_genesis() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 10).await;

        let should_reconcile = should_reconcile_l1_storage_at_startup(&storage, 10)
            .await
            .expect("check reconciliation need");

        assert!(should_reconcile);
    }

    /// Verifies reconciliation is a no-op when no stored L1 canonical tip exists.
    #[tokio::test]
    async fn startup_reconciliation_noops_without_stored_l1_tip() {
        let storage = test_storage();

        let target = reconcile_l1_canonical_storage(&storage, 10, 3, 10, |_| async {
            panic!("bitcoind should not be queried without stored L1 tip")
        })
        .await
        .expect("reconcile");

        assert_eq!(target, None);
    }

    /// Verifies pre-genesis stored L1 tips are ignored by ASM startup reconciliation.
    #[tokio::test]
    async fn startup_reconciliation_noops_when_tip_is_before_asm_genesis() {
        let storage = test_storage();
        seed_l1_chain(&storage, 3, 5).await;

        let target = reconcile_l1_canonical_storage(&storage, 10, 3, 10, |_| async {
            panic!("bitcoind should not be queried before ASM genesis")
        })
        .await
        .expect("reconcile");

        assert_eq!(target, None);
        assert_eq!(
            storage
                .l1()
                .get_canonical_chain_tip_async()
                .await
                .expect("tip")
                .expect("stored tip")
                .0,
            5
        );
    }

    /// Verifies a fully matching stored L1 tip is returned without mutation.
    #[tokio::test]
    async fn startup_reconciliation_submits_matching_l1_tip() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 15).await;

        let target = reconcile_l1_canonical_storage(&storage, 10, 3, 15, |height| async move {
            Ok(blockid(height as u8))
        })
        .await
        .expect("reconcile");

        assert_eq!(target, Some(L1BlockCommitment::new(15, blockid(15))));
        assert_eq!(
            storage
                .l1()
                .get_canonical_chain_tip_async()
                .await
                .expect("tip")
                .expect("stored tip"),
            (15, blockid(15))
        );
    }

    /// Verifies an orphaned stored L1 suffix is rewound to the matching pivot.
    #[tokio::test]
    async fn startup_reconciliation_rewinds_orphaned_l1_suffix() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 15).await;
        let bitcoind_blocks: BTreeMap<_, _> = (10..=13)
            .map(|height| (height, blockid(height as u8)))
            .chain((14..=15).map(|height| (height, blockid((height + 100) as u8))))
            .collect();

        let target = reconcile_l1_canonical_storage(&storage, 10, 3, 15, |height| {
            let bitcoind_blocks = bitcoind_blocks.clone();
            async move { Ok(*bitcoind_blocks.get(&height).expect("bitcoind height")) }
        })
        .await
        .expect("reconcile");

        assert_eq!(target, Some(L1BlockCommitment::new(13, blockid(13))));
        assert_eq!(
            storage
                .l1()
                .get_canonical_chain_tip_async()
                .await
                .expect("tip")
                .expect("stored tip"),
            (13, blockid(13))
        );
        assert!(storage
            .l1()
            .get_canonical_blockid_at_height_async(14)
            .await
            .expect("height 14")
            .is_none());
    }

    /// Verifies shallow lagging bitcoind rewinds the unseen suffix for replay.
    #[tokio::test]
    async fn startup_reconciliation_rewinds_shallow_suffix_when_bitcoind_is_behind_same_chain() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 15).await;

        let target = reconcile_l1_canonical_storage(&storage, 10, 3, 13, |height| async move {
            Ok(blockid(height as u8))
        })
        .await
        .expect("reconcile");

        assert_eq!(target, Some(L1BlockCommitment::new(13, blockid(13))));
        assert_eq!(
            storage
                .l1()
                .get_canonical_chain_tip_async()
                .await
                .expect("tip")
                .expect("stored tip"),
            (13, blockid(13))
        );
        assert!(storage
            .l1()
            .get_canonical_blockid_at_height_async(14)
            .await
            .expect("height 14")
            .is_none());
    }

    /// Verifies deep lagging bitcoind fails without mutating storage.
    #[tokio::test]
    async fn startup_reconciliation_errors_when_bitcoind_is_too_far_behind() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 15).await;

        let err = reconcile_l1_canonical_storage(&storage, 10, 3, 12, |height| async move {
            Ok(blockid(height as u8))
        })
        .await
        .expect_err("deep lag should fail startup without mutation");

        assert!(
            err.to_string().contains("too far behind stored L1"),
            "unexpected error: {err:#}"
        );
        assert_eq!(
            storage
                .l1()
                .get_canonical_chain_tip_async()
                .await
                .expect("tip")
                .expect("stored tip"),
            (15, blockid(15))
        );
    }

    /// Verifies post-genesis storage waits for bitcoind to reach ASM genesis.
    #[tokio::test]
    async fn startup_reconciliation_errors_when_bitcoind_is_before_asm_genesis() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 15).await;

        let err = reconcile_l1_canonical_storage(&storage, 10, 3, 9, |height| async move {
            Ok(blockid(height as u8))
        })
        .await
        .expect_err("bitcoind below genesis should fail startup without mutation");

        assert!(
            err.to_string().contains("too far behind stored L1"),
            "unexpected error: {err:#}"
        );
        assert_eq!(
            storage
                .l1()
                .get_canonical_chain_tip_async()
                .await
                .expect("tip")
                .expect("stored tip"),
            (15, blockid(15))
        );
    }

    /// Verifies lagging bitcoind can still trigger a safe rewind after divergence is observed.
    #[tokio::test]
    async fn startup_reconciliation_rewinds_when_lagging_bitcoind_observes_divergence() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 15).await;
        let bitcoind_blocks: BTreeMap<_, _> = [(14, blockid(114)), (13, blockid(13))]
            .into_iter()
            .collect();

        let target = reconcile_l1_canonical_storage(&storage, 10, 3, 14, |height| {
            let bitcoind_blocks = bitcoind_blocks.clone();
            async move { Ok(*bitcoind_blocks.get(&height).expect("bitcoind height")) }
        })
        .await
        .expect("reconcile");

        assert_eq!(target, Some(L1BlockCommitment::new(13, blockid(13))));
        assert_eq!(
            storage
                .l1()
                .get_canonical_chain_tip_async()
                .await
                .expect("tip")
                .expect("stored tip"),
            (13, blockid(13))
        );
    }

    /// Verifies startup fails without mutating storage when no safe pivot exists.
    #[tokio::test]
    async fn startup_reconciliation_errors_when_no_pivot_exists_in_lookback() {
        let storage = test_storage();
        seed_l1_chain(&storage, 10, 15).await;

        let err = reconcile_l1_canonical_storage(&storage, 10, 2, 15, |height| async move {
            Ok(blockid((height + 100) as u8))
        })
        .await
        .expect_err("missing pivot should fail startup");

        assert!(
            err.to_string().contains("has no bitcoind pivot"),
            "unexpected error: {err:#}"
        );
    }
}
