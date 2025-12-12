use std::sync::Arc;

use alpen_ee_block_assembly::{build_next_exec_block, BlockAssemblyInputs, BlockAssemblyOutputs};
use alpen_ee_common::{
    Clock, EnginePayload, ExecBlockPayload, ExecBlockRecord, ExecBlockStorage,
    PayloadBuilderEngine, SystemClock,
};
use alpen_ee_exec_chain::ExecChainHandle;
use eyre::Context;
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::MessageEntry;
use thiserror::Error;
use tracing::{error, info, warn};

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
fn compute_next_block_target(last_block_timestamp_ms: u64, blocktime_ms: u64) -> u64 {
    last_block_timestamp_ms + blocktime_ms
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

/// Constructs BlockAssemblyInputs from the current state.
fn create_block_assembly_inputs(
    last_local_block: &ExecBlockRecord,
    inbox_messages: Vec<MessageEntry>,
    timestamp_ms: u64,
    config: &BlockBuilderConfig,
) -> BlockAssemblyInputs {
    BlockAssemblyInputs {
        account_state: last_local_block.account_state().clone(),
        inbox_messages,
        parent_exec_blkid: last_local_block.package().exec_blkid(),
        timestamp_ms,
        max_deposits_per_block: config.max_deposits_per_block(),
        bridge_gateway_account_id: config.bridge_gateway_account_id(),
    }
}

/// Creates an ExecBlockRecord from block assembly outputs.
fn create_exec_block_record(
    package: ExecBlockPackage,
    account_state: EeAccountState,
    last_blocknum: u64,
    best_ol_block: OLBlockCommitment,
    timestamp_ms: u64,
    parent_blockhash: Hash,
) -> ExecBlockRecord {
    ExecBlockRecord::new(
        package,
        account_state,
        last_blocknum + 1,
        best_ol_block,
        timestamp_ms,
        parent_blockhash,
    )
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
                info!(?blockhash, "built new block");
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
    config: &BlockBuilderConfig,
    exec_chain_handle: &ExecChainHandle,
    ol_chain_handle: &OLChainTrackerHandle,
    payload_builder: &TEngine,
    storage: &impl ExecBlockStorage,
    clock: &impl Clock,
) -> Result<Hash, BlockBuilderError> {
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

    // submit the built payload back to engine so reth knows the block
    payload_builder
        .submit_payload(
            <TEngine::TEnginePayload as EnginePayload>::from_bytes(payload.as_bytes())
                .context("block_builder: deserialize payload")?,
        )
        .await
        .context("block_builder: submit payload to engine")?;

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

    Ok(blockhash)
}

async fn next_block_target_timestamp(
    config: &BlockBuilderConfig,
    exec_chain_handle: &ExecChainHandle,
) -> eyre::Result<u64> {
    let last_local_block = exec_chain_handle
        .get_best_block()
        .await
        .context("next_block_target_timestamp: failed to get best exec block")?;

    Ok(compute_next_block_target(
        last_local_block.timestamp_ms(),
        config.blocktime_ms(),
    ))
}

async fn build_next_block(
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

    // validate blocktime constraint
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
    let inbox_messages = if should_fetch_inbox_messages(
        last_local_block.ol_block().blkid(),
        best_ol_block.blkid(),
    ) {
        ol_chain_handle
            .get_inbox_messages(last_local_block.ol_block().slot(), best_ol_block.slot())
            .await
            .context("build_next_block: failed to get inbox messages")?
    } else {
        vec![]
    };

    // build next block
    let block_assembly_inputs =
        create_block_assembly_inputs(&last_local_block, inbox_messages, timestamp_ms, config);

    let BlockAssemblyOutputs {
        package,
        payload,
        account_state,
    } = build_next_exec_block(block_assembly_inputs, payload_builder)
        .await
        .context("build_next_block: failed to build exec block")?;

    let blockhash = package.exec_blkid();
    let parent_blockhash = last_local_block.package().exec_blkid();
    let block = create_exec_block_record(
        package,
        account_state,
        last_local_block.blocknum(),
        best_ol_block,
        timestamp_ms,
        parent_blockhash,
    );

    Ok((block, payload, blockhash))
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_ee_chain_types::{BlockInputs, BlockOutputs, ExecBlockCommitment};
    use strata_identifiers::Buf32;

    use super::*;

    /// Helper to create an OLBlockCommitment with a given slot.
    fn make_ol_block(slot: u64) -> OLBlockCommitment {
        let mut blkid_bytes = [0u8; 32];
        blkid_bytes[0..8].copy_from_slice(&slot.to_le_bytes());
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from(blkid_bytes)))
    }

    /// Helper to create a test ExecBlockRecord.
    fn make_exec_block_record(
        blocknum: u64,
        timestamp_ms: u64,
        ol_block: OLBlockCommitment,
    ) -> ExecBlockRecord {
        let hash = Hash::from(Buf32::new([blocknum as u8; 32]));
        let package = ExecBlockPackage::new(
            ExecBlockCommitment::new(hash, hash),
            BlockInputs::new_empty(),
            BlockOutputs::new_empty(),
        );
        let account_state = EeAccountState::new(hash, BitcoinAmount::ZERO, vec![], vec![]);
        ExecBlockRecord::new(
            package,
            account_state,
            blocknum,
            ol_block,
            timestamp_ms,
            Hash::default(),
        )
    }

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

    mod create_block_assembly_inputs_tests {
        use strata_acct_types::{AccountId, MsgPayload};

        use super::*;

        #[test]
        fn preserves_message_order() {
            // Message order matters for deterministic block assembly
            let ol_block = make_ol_block(10);
            let exec_record = make_exec_block_record(5, 5000, ol_block);
            let config = BlockBuilderConfig::default();

            let msg1 = MessageEntry::new(
                AccountId::new([1u8; 32]),
                0,
                MsgPayload::new(BitcoinAmount::from_sat(100), vec![]),
            );
            let msg2 = MessageEntry::new(
                AccountId::new([2u8; 32]),
                0,
                MsgPayload::new(BitcoinAmount::from_sat(200), vec![]),
            );
            let messages = vec![msg1.clone(), msg2.clone()];

            let inputs = create_block_assembly_inputs(&exec_record, messages, 6000, &config);

            assert_eq!(inputs.inbox_messages.len(), 2);
            assert_eq!(inputs.inbox_messages[0].source(), msg1.source());
            assert_eq!(inputs.inbox_messages[1].source(), msg2.source());
        }
    }
}
