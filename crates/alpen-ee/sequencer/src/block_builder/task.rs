use std::sync::Arc;

use alpen_ee_block_assembly::{build_next_exec_block, BlockAssemblyInputs, BlockAssemblyOutputs};
use alpen_ee_common::{
    Clock, EnginePayload, ExecBlockPayload, ExecBlockRecord, ExecBlockStorage,
    PayloadBuilderEngine, SystemClock,
};
use alpen_ee_exec_chain::ExecChainHandle;
use eyre::Context;
use strata_acct_types::{Hash, MessageEntry};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
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

/// Picks the OL block commitment to record on the next exec block.
///
/// During restart catch-up, finalization lag, or reorg handling, the OL
/// tracker can report a block behind the local EE tip. Returning
/// `last_local_ol_block` in that case keeps the OL slot recorded on the EE
/// chain monotonic; using the older `best_ol_block` would cause inbox
/// fetching to re-include slots already reflected in account state.
fn effective_ol_block(
    best_ol_block: OLBlockCommitment,
    last_local_ol_block: OLBlockCommitment,
) -> OLBlockCommitment {
    if best_ol_block.slot() < last_local_ol_block.slot() {
        last_local_ol_block
    } else {
        best_ol_block
    }
}

/// Computes the inbox-fetch slot range for the next exec block.
///
/// Returns `None` when no fetch is needed: either the OL tip is unchanged
/// from what's already in account state, or the slot delta is empty.
/// Otherwise returns `Some((from_slot, to_slot))` with both ends inclusive.
///
/// `from_slot = last_local_ol_slot + 1` is the off-by-one fix: the previously
/// recorded slot's messages are already drained, so re-fetching it produces a
/// SAU whose `processed_messages` does not match `new_next_inbox_msg_idx` and
/// OL rejects it with "invalid next msg index".
fn compute_inbox_fetch_range(
    last_local_ol_slot: u64,
    last_local_ol_blkid: &OLBlockId,
    effective_ol_slot: u64,
    effective_ol_blkid: &OLBlockId,
) -> Option<(u64, u64)> {
    if !should_fetch_inbox_messages(last_local_ol_blkid, effective_ol_blkid) {
        return None;
    }
    let from_slot = last_local_ol_slot.saturating_add(1);
    if from_slot > effective_ol_slot {
        return None;
    }
    Some((from_slot, effective_ol_slot))
}

/// Constructs BlockAssemblyInputs from the current state.
fn create_block_assembly_inputs<'a>(
    last_local_block: &ExecBlockRecord,
    inbox_messages: &'a [MessageEntry],
    timestamp_ms: u64,
    config: &BlockBuilderConfig,
) -> BlockAssemblyInputs<'a> {
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
#[expect(clippy::too_many_arguments, reason = "too many args")]
fn create_next_exec_block_record(
    package: ExecBlockPackage,
    account_state: EeAccountState,
    last_blocknum: u64,
    best_ol_block: OLBlockCommitment,
    timestamp_ms: u64,
    parent_blockhash: Hash,
    next_inbox_msg_idx: u64,
    messages: Vec<MessageEntry>,
) -> ExecBlockRecord {
    ExecBlockRecord::new(
        package,
        account_state,
        last_blocknum + 1,
        best_ol_block,
        timestamp_ms,
        parent_blockhash,
        next_inbox_msg_idx,
        messages,
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

    // Check the parent expected by the block target. A mismatch means another
    // builder advanced the local EE tip.
    if last_local_block.blockhash() != expected_block_target.parent {
        warn!(
            expected = %expected_block_target.parent,
            actual = %last_local_block.blockhash(),
            "build_next_block: unexpected latest blockhash"
        )
    }

    // Enforce the configured blocktime against the current local tip.
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

    let effective_ol_block = effective_ol_block(best_ol_block, *last_local_block.ol_block());

    let (inbox_messages, next_inbox_msg_idx) = match compute_inbox_fetch_range(
        last_local_block.ol_block().slot(),
        last_local_block.ol_block().blkid(),
        effective_ol_block.slot(),
        effective_ol_block.blkid(),
    ) {
        Some((from_slot, to_slot)) => ol_chain_handle
            .get_inbox_messages(from_slot, to_slot)
            .await
            .context("build_next_block: failed to get inbox messages")?
            .into_parts(),
        None => (vec![], last_local_block.next_inbox_msg_idx()),
    };

    // build next block
    let block_assembly_inputs =
        create_block_assembly_inputs(&last_local_block, &inbox_messages, timestamp_ms, config);

    let BlockAssemblyOutputs {
        package,
        payload,
        account_state,
    } = build_next_exec_block(block_assembly_inputs, payload_builder)
        .await
        .context("build_next_block: failed to build exec block")?;

    let blockhash = package.exec_blkid();
    let parent_blockhash = last_local_block.package().exec_blkid();
    let block = create_next_exec_block_record(
        package,
        account_state,
        last_local_block.blocknum(),
        effective_ol_block,
        timestamp_ms,
        parent_blockhash,
        next_inbox_msg_idx,
        inbox_messages,
    );

    Ok((block, payload, blockhash))
}

#[cfg(test)]
mod tests {
    use std::vec;

    use strata_acct_types::BitcoinAmount;
    use strata_ee_chain_types::{ExecBlockCommitment, ExecInputs, ExecOutputs};
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
            ExecInputs::new_empty(),
            ExecOutputs::new_empty(),
        );
        let account_state = EeAccountState::new(hash, BitcoinAmount::ZERO, vec![], vec![]);
        ExecBlockRecord::new(
            package,
            account_state,
            blocknum,
            ol_block,
            timestamp_ms,
            Hash::default(),
            0,
            vec![],
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

            let inputs = create_block_assembly_inputs(&exec_record, &messages, 6000, &config);

            assert_eq!(inputs.inbox_messages.len(), 2);
            assert_eq!(inputs.inbox_messages[0].source(), msg1.source());
            assert_eq!(inputs.inbox_messages[1].source(), msg2.source());
        }
    }

    mod effective_ol_block_tests {
        use super::*;

        #[test]
        fn picks_best_when_best_is_ahead() {
            let last = make_ol_block(5);
            let best = make_ol_block(10);
            assert_eq!(effective_ol_block(best, last), best);
        }

        #[test]
        fn picks_best_when_slots_equal() {
            // Equal slots: best wins. Different blkid at the same slot still
            // means the OL tracker advanced (e.g. reorg sibling).
            let last = make_ol_block(5);
            let best = make_ol_block(5);
            assert_eq!(effective_ol_block(best, last), best);
        }

        #[test]
        fn falls_back_to_last_when_best_is_behind() {
            // Restart catch-up / finalization lag: tracker behind the local
            // tip. Falling back to `last` keeps the recorded OL slot
            // monotonic; using `best` here would re-include drained slots.
            let last = make_ol_block(10);
            let best = make_ol_block(5);
            assert_eq!(effective_ol_block(best, last), last);
        }
    }

    mod compute_inbox_fetch_range_tests {
        use super::*;

        fn blkid(byte: u8) -> OLBlockId {
            let mut bytes = [0u8; 32];
            bytes[31] = byte;
            OLBlockId::from(Buf32::from(bytes))
        }

        #[test]
        fn returns_none_when_blkids_match() {
            // OL tip unchanged: nothing new to fetch even if slots happened
            // to differ (would mean a slot was rewritten in place).
            let id = blkid(1);
            assert_eq!(compute_inbox_fetch_range(5, &id, 5, &id), None);
        }

        #[test]
        fn returns_none_when_range_is_empty() {
            // Different blkids but `from > to` — can happen if `effective_ol`
            // falls back to `last_local` after a behind-tracker case and the
            // siblings carry the same slot but different ids. No slots to
            // pull.
            assert_eq!(compute_inbox_fetch_range(5, &blkid(1), 5, &blkid(2)), None);
        }

        #[test]
        fn fetches_only_new_slots_off_by_one_regression() {
            // Last local OL block at slot 5, new effective at slot 7.
            // The fetch must start at slot 6, NOT slot 5: re-including slot 5
            // would re-process messages already drained into account state
            // and OL would reject the resulting SAU with "invalid next msg
            // index".
            assert_eq!(
                compute_inbox_fetch_range(5, &blkid(1), 7, &blkid(2)),
                Some((6, 7)),
            );
        }

        #[test]
        fn single_slot_advance_returns_singleton_range() {
            // Slot 5 -> slot 6: fetch [6, 6].
            assert_eq!(
                compute_inbox_fetch_range(5, &blkid(1), 6, &blkid(2)),
                Some((6, 6)),
            );
        }
    }
}
