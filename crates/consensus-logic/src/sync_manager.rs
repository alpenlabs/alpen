//! High level sync manager which controls core sync tasks and manages sync
//! status.  Exposes handles to interact with fork choice manager and CSM
//! executor and other core sync pipeline tasks.

use std::sync::Arc;

use anyhow::Context;
use bitcoind_async_client::Client;
use strata_asm_params::AsmParams;
use strata_asm_spec::StrataAsmSpec;
#[cfg(feature = "debug-asm")]
use strata_asm_spec_debug::DebugAsmSpec;
use strata_asm_worker::{AsmState as WorkerAsmState, AsmWorkerHandle, AsmWorkerStatus};
use strata_btc_types::L1BlockIdBitcoinExt;
use strata_csm_worker::{CsmWorkerService, CsmWorkerState, CsmWorkerStatus};
use strata_node_context::NodeContext;
use strata_ol_state_types::MMR_SENTINEL_DUMMY_LEAF_HASH;
use strata_primitives::prelude::{L1BlockCommitment, L1Height};
use strata_service::{ServiceBuilder, ServiceMonitor, SyncAsyncInput};
use strata_state::asm_state::AsmState as StorageAsmState;
use strata_status::StatusChannel;
use strata_storage::{MmrId, MmrIndexHandle, NodeStorage};
use strata_tasks::TaskExecutor;
use tokio::runtime::Handle;
use tracing::{debug, warn};

use crate::{asm_worker_context::AsmWorkerCtx, csm_worker_context::CsmWorkerContextImpl};

pub fn spawn_csm_listener_with_ctx(
    nodectx: &NodeContext,
    asm_monitor: &ServiceMonitor<AsmWorkerStatus>,
) -> anyhow::Result<ServiceMonitor<CsmWorkerStatus>> {
    spawn_csm_listener(
        nodectx.executor(),
        nodectx.asm_params().clone(),
        nodectx.config().btcio.l1_reorg_safe_depth,
        nodectx.storage().clone(),
        nodectx.status_channel().clone(),
        asm_monitor,
        nodectx.bitcoin_client().clone(),
    )
}

fn spawn_csm_listener(
    executor: &TaskExecutor,
    asm_params: Arc<AsmParams>,
    l1_reorg_safe_depth: u32,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    asm_monitor: &ServiceMonitor<AsmWorkerStatus>,
    bitcoin_client: Arc<Client>,
) -> anyhow::Result<ServiceMonitor<CsmWorkerStatus>> {
    // Create CSM worker state.
    let ctx = CsmWorkerContextImpl::new(
        executor.handle().clone(),
        bitcoin_client,
        asm_params,
        l1_reorg_safe_depth,
        storage.clone(),
        status_channel,
    );
    let csm_state = CsmWorkerState::init_from_context(ctx)?;

    // Get the starting block from CSM's last processed block
    // If CSM hasn't processed any blocks yet, we get the latest ASM state from storage
    let from_block = if let Some(last_block) = csm_state.get_last_asm_block() {
        last_block
    } else {
        // Get the latest ASM state as fallback
        let (latest_block, _) = storage
            .asm()
            .fetch_most_recent_state_blocking()?
            .expect("No ASM state available");
        latest_block
    };

    // Fetch historical ASM states starting from the next height.
    let max_historical_blocks = 1000;
    let nh = from_block.height() + 1;
    let historical_states = storage.asm().get_states_from_blocking(
        L1BlockCommitment::new(nh, Default::default()),
        max_historical_blocks,
    )?;

    // Convert historical states to ASM worker status updates
    let initial_updates: Vec<AsmWorkerStatus> = historical_states
        .into_iter()
        .map(|(block, state)| AsmWorkerStatus {
            is_initialized: true,
            cur_block: Some(block),
            cur_state: Some(storage_to_worker_state(state)),
        })
        .collect();

    // Create an input that listens to ASM status updates with historical prepended
    let async_input = asm_monitor.create_listener_input_with(executor, initial_updates);
    // Wrap in SyncAsyncInput adapter since CSM worker is a sync service.
    let csm_input = SyncAsyncInput::new(async_input, executor.handle().clone());

    // Launch the CSM worker service (which acts as a listener to ASM worker).
    let csm_monitor = ServiceBuilder::<CsmWorkerService<CsmWorkerContextImpl>, _>::new()
        .with_state(csm_state)
        .with_input(csm_input)
        .launch_sync("csm_worker", executor)?;

    Ok(csm_monitor)
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

    resubmit_canonical_tip_if_asm_behind(storage.as_ref(), &handle, genesis_l1_height as L1Height)?;

    Ok(handle)
}

fn storage_to_worker_state(state: StorageAsmState) -> WorkerAsmState {
    WorkerAsmState::new(state.state().clone(), state.logs().clone())
}

/// Submits the stored canonical L1 tip to ASM when persisted ASM state lags it.
///
/// btcio writes canonical L1 chain entries before it submits blocks to ASM. If
/// the process crashes after the canonical write but before ASM stores the
/// corresponding anchor state, the reader resumes after that canonical tip and
/// no new L1 event is emitted until another Bitcoin block arrives. Submitting
/// the stored tip during ASM startup lets the ASM worker backfill from its
/// latest anchor state through the canonical chain without making btcio inspect
/// ASM-specific storage.
fn resubmit_canonical_tip_if_asm_behind(
    storage: &NodeStorage,
    asm_handle: &AsmWorkerHandle,
    genesis_l1_height: L1Height,
) -> anyhow::Result<()> {
    let Some((tip_height, tip_blockid)) = storage.l1().get_canonical_chain_tip()? else {
        return Ok(());
    };

    if tip_height < genesis_l1_height {
        return Ok(());
    }

    let latest_asm_block = storage
        .asm()
        .fetch_most_recent_state_blocking()?
        .map(|(block, _)| block);
    let tip = L1BlockCommitment::new(tip_height, tip_blockid);

    if latest_asm_block.as_ref().is_some_and(|block| block == &tip) {
        debug!(%tip, "canonical L1 tip already has ASM state");
        return Ok(());
    }

    warn!(
        %tip,
        latest_asm_block = latest_asm_block
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
            .unwrap_or("none"),
        "canonical L1 tip is ahead of ASM state; re-submitting tip to ASM for startup backfill"
    );
    asm_handle
        .submit_block(tip.blkid().to_block_hash())
        .with_context(|| format!("failed to re-submit canonical L1 tip {tip} to ASM"))?;

    Ok(())
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
