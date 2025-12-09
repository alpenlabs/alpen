use std::{future::Future, sync::Arc, time::Duration};

use alpen_ee_block_assembly::{build_next_exec_block, BlockAssemblyInputs, BlockAssemblyOutputs};
use alpen_ee_common::{ExecBlockPayload, ExecBlockRecord, ExecBlockStorage, PayloadBuilderEngine};
use alpen_ee_exec_chain::ExecChainHandle;
use eyre::Context;
use strata_acct_types::Hash;
use tracing::{debug, error};

use crate::{block_builder::BlockBuilderConfig, ol_chain_tracker::OLChainTrackerHandle};

trait Clock: Sized {
    /// current time in milliseconds since UNIX_EPOCH
    fn current_timestamp(&self) -> u64;
    /// sleep until unix timestamp
    fn sleep_until(&self, timestamp_ms: u64) -> impl Future<Output = ()>;
}

struct SystemClock;

impl Clock for SystemClock {
    fn current_timestamp(&self) -> u64 {
        std::time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64
    }

    fn sleep_until(&self, timestamp_ms: u64) -> impl Future<Output = ()> {
        let now = self.current_timestamp();
        tokio::time::sleep(Duration::from_millis(timestamp_ms.saturating_sub(now)))
    }
}

pub async fn block_builder_task<
    TPayloadBuilder: PayloadBuilderEngine,
    TStorage: ExecBlockStorage,
>(
    config: BlockBuilderConfig,
    exec_chain_handle: ExecChainHandle,
    ol_chain_handle: OLChainTrackerHandle,
    payload_builder: Arc<TPayloadBuilder>,
    storage: Arc<TStorage>,
) {
    let clock = SystemClock;
    loop {
        match block_builder_task_inner(
            &config,
            &exec_chain_handle,
            &ol_chain_handle,
            payload_builder.as_ref(),
            storage.as_ref(),
            &clock,
        )
        .await
        {
            Ok(blockhash) => {
                debug!(?blockhash, "built new block");
            }
            Err(err) => {
                error!(?err, "failed to build block");
            }
        }
    }
}

async fn block_builder_task_inner(
    config: &BlockBuilderConfig,
    exec_chain_handle: &ExecChainHandle,
    ol_chain_handle: &OLChainTrackerHandle,
    payload_builder: &impl PayloadBuilderEngine,
    storage: &impl ExecBlockStorage,
    clock: &impl Clock,
) -> eyre::Result<Hash> {
    // check when the next block should be built
    let next_block_target = next_block_target_timestamp(config, exec_chain_handle).await?;

    // if we are not ready, sleep
    clock.sleep_until(next_block_target).await;

    // we can build blocks now
    let (block, payload, blockhash) = build_next_block(
        config,
        exec_chain_handle,
        ol_chain_handle,
        payload_builder,
        clock,
    )
    .await?;

    // save block outputs
    storage
        .save_exec_block(block, payload)
        .await
        .context("failed to save exec block")?;
    // submit block to chain tracker
    exec_chain_handle
        .new_block(blockhash)
        .await
        .context("failed to submit new exec block")?;

    Ok(blockhash)
}

async fn next_block_target_timestamp(
    config: &BlockBuilderConfig,
    exec_chain_handle: &ExecChainHandle,
) -> eyre::Result<u64> {
    let last_local_block = exec_chain_handle
        .get_best_block()
        .await
        .context("failed to get best exec block")?;

    Ok(last_local_block.timestamp_ms() + config.blocktime_ms)
}

async fn build_next_block(
    config: &BlockBuilderConfig,
    exec_chain_handle: &ExecChainHandle,
    ol_chain_handle: &OLChainTrackerHandle,
    payload_builder: &impl PayloadBuilderEngine,
    clock: &impl Clock,
) -> eyre::Result<(ExecBlockRecord, ExecBlockPayload, Hash)> {
    let last_local_block = exec_chain_handle
        .get_best_block()
        .await
        .context("failed to get best exec block")?;
    // check if there are new OL block inputs that need to be included
    let best_ol_block = ol_chain_handle
        .get_finalized_block()
        .await
        .context("failed to get finalized OL block")?;
    let inbox_messages = if last_local_block.ol_block().blkid() != best_ol_block.blkid() {
        ol_chain_handle
            .get_inbox_messages(last_local_block.ol_block().slot(), best_ol_block.slot())
            .await
            .context("failed to get inbox messages")?
    } else {
        vec![]
    };

    // build next block
    let timestamp_ms = clock.current_timestamp();
    let parent_blockhash = last_local_block.package().exec_blkid();
    let block_assembly_inputs = BlockAssemblyInputs {
        account_state: last_local_block.account_state().clone(),
        inbox_messages,
        parent_exec_blkid: parent_blockhash,
        timestamp_ms,
        max_deposits_per_block: config.max_deposits_per_block,
        bridge_gateway_account_id: config.bridge_gateway_account_id,
    };

    let BlockAssemblyOutputs {
        package,
        payload,
        account_state,
    } = build_next_exec_block(block_assembly_inputs, payload_builder)
        .await
        .context("failed to build exec block")?;

    let blockhash = package.exec_blkid();
    let blocknum = last_local_block.blocknum() + 1;
    let block = ExecBlockRecord::new(
        package,
        account_state,
        blocknum,
        best_ol_block,
        timestamp_ms,
        parent_blockhash,
    );

    Ok((block, payload, blockhash))
}
