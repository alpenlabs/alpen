use std::sync::Arc;

use alpen_ee_block_assembly::{
    assemble_next_exec_block_record, AssembleExecBlockInputs, AssembledExecBlock,
};
use alpen_ee_common::{
    Clock, EnginePayload, ExecBlockPayload, ExecBlockRecord, ExecBlockStorage,
    PayloadBuilderEngine, SystemClock,
};
use alpen_ee_exec_chain::ExecChainHandle;
use eyre::Context;
use strata_acct_types::Hash;
use strata_identifiers::OLBlockId;
use thiserror::Error;
use tracing::{debug, error, warn};

use crate::{block_builder::BlockBuilderConfig, ol_chain_tracker::OLChainTrackerHandle};

/// Error type for block builder that distinguishes retriable from real errors.
#[derive(Debug, Error)]
enum BlockBuilderError {
    /// Timestamp constraint violated - should retry immediately without backoff.
    #[error("blocktime constraint violated")]
    BlocktimeConstraintViolated,
    /// Real error occurred - should backoff before retry.
    #[error(transparent)]
    Other(#[from] eyre::Report),
}

/// Computes the target timestamp for the next block.
fn compute_next_block_target(
    last_block: &ExecBlockRecord,
    config: &BlockBuilderConfig,
) -> BlockTarget {
    BlockTarget {
        parent: last_block.blockhash(),
        timestamp_ms: last_block.timestamp_ms() + config.blocktime_ms(),
    }
}

/// Validates that current time meets the minimum blocktime constraint.
fn validate_blocktime_constraint(
    current_timestamp_ms: u64,
    last_block_timestamp_ms: u64,
    blocktime_ms: u64,
) -> Result<(), BlockBuilderError> {
    let min_timestamp = last_block_timestamp_ms + blocktime_ms;
    if current_timestamp_ms < min_timestamp {
        return Err(BlockBuilderError::BlocktimeConstraintViolated);
    }
    Ok(())
}

/// Determines whether inbox messages should be fetched based on OL block state.
fn should_fetch_inbox_messages(last_local_ol_blkid: &OLBlockId, best_ol_blkid: &OLBlockId) -> bool {
    last_local_ol_blkid != best_ol_blkid
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
    let last_local_block = exec_chain_handle
        .get_best_block()
        .await
        .expect("next_block_target_timestamp: failed to get best exec block");
    let last_hash = last_local_block.parent_blockhash();
    debug!(%last_hash, "last local block parent");

    let mut next_block_target = compute_next_block_target(&last_local_block, &config);
    debug!(?next_block_target, "next block target");

    let clock = SystemClock;
    loop {
        match block_builder_task_inner(
            &next_block_target,
            &config,
            &exec_chain_handle,
            &ol_chain_handle,
            payload_builder.as_ref(),
            storage.as_ref(),
            &clock,
        )
        .await
        {
            Ok((blockhash, next_target)) => {
                debug!(?blockhash, "built new block");
                next_block_target = next_target;
            }
            Err(BlockBuilderError::BlocktimeConstraintViolated) => {
                warn!("blocktime constraint violated, retrying immediately");
            }
            Err(BlockBuilderError::Other(err)) => {
                error!(?err, "failed to build block");
                clock.sleep_ms(config.error_backoff_ms()).await;
            }
        }
    }
}

async fn block_builder_task_inner<TEngine: PayloadBuilderEngine>(
    next_block_target: &BlockTarget,
    config: &BlockBuilderConfig,
    exec_chain_handle: &ExecChainHandle,
    ol_chain_handle: &OLChainTrackerHandle,
    payload_builder: &TEngine,
    storage: &impl ExecBlockStorage,
    clock: &impl Clock,
) -> Result<(Hash, BlockTarget), BlockBuilderError> {
    // if we are not ready, sleep
    clock.sleep_until(next_block_target.timestamp_ms).await;

    // we can build blocks now
    let (block, payload, blockhash) = build_next_block(
        next_block_target,
        config,
        exec_chain_handle,
        ol_chain_handle,
        payload_builder,
        clock,
    )
    .await?;

    // submit the built payload back to engine so reth knows the block
    payload_builder
        .submit_payload(
            <TEngine::TEnginePayload as EnginePayload>::from_bytes(payload.as_bytes())
                .context("block_builder: deserialize payload")?,
        )
        .await
        .context("block_builder: submit payload to engine")?;

    // cache next block target
    let next_block_target = compute_next_block_target(&block, config);

    // save block outputs
    storage
        .save_exec_block(block, payload)
        .await
        .context("block_builder: save exec block to storage")?;

    // submit block to chain tracker
    exec_chain_handle
        .new_block(blockhash)
        .await
        .context("block_builder: submit new exec block")?;

    // TODO: should this wait for block

    Ok((blockhash, next_block_target))
}

// Next Block building target data
#[derive(Debug)]
struct BlockTarget {
    parent: Hash,
    timestamp_ms: u64,
}

async fn build_next_block(
    expected_block_target: &BlockTarget,
    config: &BlockBuilderConfig,
    exec_chain_handle: &ExecChainHandle,
    ol_chain_handle: &OLChainTrackerHandle,
    payload_builder: &impl PayloadBuilderEngine,
    clock: &impl Clock,
) -> Result<(ExecBlockRecord, ExecBlockPayload, Hash), BlockBuilderError> {
    let last_local_block = exec_chain_handle
        .get_best_block()
        .await
        .context("build_next_block: failed to get best exec block")?;

    // Check if last local block is not as expected from previous block building cycle
    // This shouldn't happen in a single sequencer case, but checking for sanity anyway.
    if last_local_block.blockhash() != expected_block_target.parent {
        warn!(
            expected = %expected_block_target.parent,
            actual = %last_local_block.blockhash(),
            "build_next_block: unexpected latest blockhash"
        )
    }

    // Ensure blocktime >= configured blocktime
    // This shouldn't happen in a single sequencer case, but checking for sanity anyway.
    let timestamp_ms = clock.current_timestamp();
    validate_blocktime_constraint(
        timestamp_ms,
        last_local_block.timestamp_ms(),
        config.blocktime_ms(),
    )?;

    // check if there are new OL block inputs that need to be included
    let best_ol_block = ol_chain_handle
        .get_finalized_block()
        .await
        .context("build_next_block: failed to get finalized OL block")?;
    let (inbox_messages, next_inbox_msg_idx) = if should_fetch_inbox_messages(
        last_local_block.ol_block().blkid(),
        best_ol_block.blkid(),
    ) {
        ol_chain_handle
            .get_inbox_messages(last_local_block.ol_block().slot(), best_ol_block.slot())
            .await
            .context("build_next_block: failed to get inbox messages")?
            .into_parts()
    } else {
        (vec![], last_local_block.next_inbox_msg_idx())
    };

    let AssembledExecBlock {
        record,
        payload,
        blockhash,
    } = assemble_next_exec_block_record(
        AssembleExecBlockInputs {
            parent_record: &last_local_block,
            inbox_messages,
            next_inbox_msg_idx,
            best_ol_block,
            timestamp_ms,
            max_deposits_per_block: config.max_deposits_per_block(),
            bridge_gateway_account_id: config.bridge_gateway_account_id(),
        },
        payload_builder,
    )
    .await
    .context("build_next_block: failed to assemble exec block")?;

    Ok((record, payload, blockhash))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod validate_blocktime_constraint_tests {
        use super::*;

        #[test]
        fn boundary_exactly_at_min_timestamp_succeeds() {
            // Edge case: current time equals exactly last_block + blocktime
            // This is the minimum valid time - should pass
            let result = validate_blocktime_constraint(2000, 1000, 1000);
            assert!(result.is_ok());
        }

        #[test]
        fn fails_when_clock_appears_to_go_backwards() {
            // Defensive: if current time is before last block (clock drift/reset),
            // should fail the constraint check
            let result = validate_blocktime_constraint(500, 1000, 1000);
            assert!(matches!(
                result,
                Err(BlockBuilderError::BlocktimeConstraintViolated)
            ));
        }
    }
}
