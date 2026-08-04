//! Resolves the proof-backed recovery target from a synced OL node.

use alloy_primitives::{hex, B256};
use anyhow::{anyhow, bail, Context, Result};
use jsonrpsee::http_client::HttpClientBuilder;
use strata_acct_types::{AccountId, Hash, MessageEntry};
use strata_codec::decode_buf_exact;
use strata_ee_acct_runtime::apply_input_messages;
use strata_ee_acct_types::{EeAccountState, PendingFinclEntry, PendingInputEntry, UpdateExtraData};
use strata_identifiers::EpochCommitment;
use strata_ol_rpc_api::OLClientRpcClient;
use strata_ol_rpc_types::OLBlockTag;

/// Proof-backed OL coordinates used by state reconstruction and bootstrap.
#[derive(Debug)]
pub(super) struct OlRecoveryTarget {
    pub(super) last_exec_blkid: B256,
    pub(super) expected_inner_state_root: B256,
    pub(super) finalized_epoch: EpochCommitment,
    pub(super) previous_batch_block_hash: B256,
    pub(super) next_inbox_msg_idx: u64,
    pub(super) next_deposit_idx: u64,
    pub(super) pending_inputs: Vec<PendingInputEntry>,
    pub(super) pending_fincls: Vec<PendingFinclEntry>,
}

fn parse_account_id(value: &str) -> Result<AccountId> {
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(value.trim().trim_start_matches("0x"), &mut bytes)
        .context("parsing EE account ID")?;
    Ok(AccountId::new(bytes))
}

fn apply_queue_transition(
    state: &mut EeAccountState,
    messages: &[MessageEntry],
    extra_data: &UpdateExtraData,
    sequence: u64,
) -> Result<()> {
    apply_input_messages(state, messages)
        .map_err(|error| anyhow!("applying OL messages for update sequence {sequence}: {error}"))?;

    let processed_inputs = usize::try_from(*extra_data.processed_inputs())
        .context("processed input count exceeds usize")?;
    if processed_inputs > state.pending_inputs().len() {
        bail!(
            "OL update sequence {sequence} processes {processed_inputs} inputs, but only {} are \
             pending",
            state.pending_inputs().len()
        );
    }
    let removed_inputs = state.remove_pending_inputs(processed_inputs);
    if removed_inputs.len() != processed_inputs {
        bail!("failed to drain the validated pending-input count");
    }

    let processed_fincls = usize::try_from(*extra_data.processed_fincls())
        .context("processed forced-inclusion count exceeds usize")?;
    if processed_fincls > state.pending_fincls().len() {
        bail!(
            "OL update sequence {sequence} processes {processed_fincls} forced inclusions, but \
             only {} are pending; the current EE runtime has no forced-inclusion producer",
            state.pending_fincls().len()
        );
    }
    let removed_fincls = state.remove_pending_fincls(processed_fincls);
    if removed_fincls.len() != processed_fincls {
        bail!("failed to drain the validated pending forced-inclusion count");
    }

    Ok(())
}

fn validate_anchor_inbox_cursor(recovered_cursor: u64, anchor_inbox_leaf_count: u64) -> Result<()> {
    if anchor_inbox_leaf_count < recovered_cursor {
        bail!(
            "finalized OL anchor inbox has {anchor_inbox_leaf_count} leaves, fewer than recovered \
             cursor {recovered_cursor}"
        );
    }
    if anchor_inbox_leaf_count > recovered_cursor {
        bail!(
            "finalized OL anchor contains inbox messages at or after recovered cursor \
             {recovered_cursor}; cursor-based OL catch-up is required"
        );
    }

    Ok(())
}

/// Loads an accepted update and requires it to be the OL-finalized EE state.
pub(super) async fn load_recovery_target(
    rpc_url: &str,
    account_id: &str,
    target_update_seq_no: u64,
    genesis_exec_blkid: B256,
) -> Result<OlRecoveryTarget> {
    let account_id = parse_account_id(account_id)?;
    let client = HttpClientBuilder::default()
        .build(rpc_url)
        .with_context(|| format!("creating OL RPC client for {rpc_url}"))?;

    let mut target_manifest = None;
    let mut target_extra_data = None;
    let mut previous_batch_block_hash = genesis_exec_blkid;
    let mut next_deposit_idx = 0u64;
    let mut expected_inbox_cursor = 0u64;
    let mut queue_state = EeAccountState::new(Hash::zero(), Hash::zero(), Vec::new(), Vec::new());
    for sequence in 0..=target_update_seq_no {
        let manifest = client
            .get_snark_acct_update_manifest(account_id, sequence)
            .await
            .with_context(|| format!("loading OL update manifest for sequence {sequence}"))?;
        if manifest.seq_no() != sequence {
            bail!(
                "OL returned update sequence {} while sequence {sequence} was requested",
                manifest.seq_no()
            );
        }
        if manifest.prev_next_msg_idx() != expected_inbox_cursor {
            bail!(
                "OL update sequence {sequence} starts at inbox cursor {}, expected \
                 {expected_inbox_cursor}",
                manifest.prev_next_msg_idx()
            );
        }
        let next_inbox_cursor = manifest.new_next_msg_idx();
        if next_inbox_cursor < expected_inbox_cursor {
            bail!(
                "OL update sequence {sequence} moves inbox cursor backwards from \
                 {expected_inbox_cursor} to {next_inbox_cursor}"
            );
        }
        let rpc_messages = client
            .get_snark_acct_inbox_msg_range(account_id, expected_inbox_cursor, next_inbox_cursor)
            .await
            .with_context(|| {
                format!("loading consumed inbox messages for OL update sequence {sequence}")
            })?;
        let expected_message_count = usize::try_from(next_inbox_cursor - expected_inbox_cursor)
            .context("consumed inbox range exceeds usize")?;
        if rpc_messages.len() != expected_message_count {
            bail!(
                "OL update sequence {sequence} consumes {expected_message_count} messages, but RPC \
                 returned {}",
                rpc_messages.len()
            );
        }
        let mut messages = Vec::with_capacity(rpc_messages.len());
        for (offset, rpc_message) in rpc_messages.into_iter().enumerate() {
            let expected_index = expected_inbox_cursor
                .checked_add(u64::try_from(offset).context("inbox message offset overflow")?)
                .context("inbox message index overflow")?;
            if rpc_message.index() != expected_index {
                bail!(
                    "OL update sequence {sequence} returned inbox index {}, expected \
                     {expected_index}",
                    rpc_message.index()
                );
            }
            messages.push(
                MessageEntry::try_from(rpc_message.value().clone()).with_context(|| {
                    format!(
                        "decoding inbox message {expected_index} for OL update sequence {sequence}"
                    )
                })?,
            );
        }
        let extra_data = manifest
            .extra_data()
            .with_context(|| format!("OL update sequence {sequence} has no EE extra data"))?;
        let decoded: UpdateExtraData = decode_buf_exact(&extra_data.0)
            .with_context(|| format!("decoding EE extra data for OL update sequence {sequence}"))?;
        apply_queue_transition(&mut queue_state, &messages, &decoded, sequence)?;
        // PendingInputEntry currently has only the Deposit variant, so the
        // proof-bound processed-input count is also the executed-deposit count.
        next_deposit_idx = next_deposit_idx
            .checked_add(u64::from(*decoded.processed_inputs()))
            .context("reconstructed deposit index overflow")?;

        if sequence.checked_add(1) == Some(target_update_seq_no) {
            previous_batch_block_hash = B256::from(decoded.new_tip_blkid().0);
        }
        if sequence == target_update_seq_no {
            target_extra_data = Some(decoded);
            target_manifest = Some(manifest);
        }
        expected_inbox_cursor = next_inbox_cursor;
    }
    let (_, _, pending_inputs, pending_fincls) = queue_state.into_parts();
    let manifest = target_manifest.expect("target sequence is included in the requested range");
    let decoded = target_extra_data.expect("target sequence is included in the requested range");
    let expected_inner_state_root = manifest
        .new_inner_state_root()
        .context("OL update manifest does not expose its proof-backed inner state root")?;
    let next_inbox_msg_idx = manifest.new_next_msg_idx();

    let finalized_state = client
        .get_snark_account_state_by_tag(account_id, OLBlockTag::Finalized)
        .await
        .context("loading finalized EE account state from OL")?
        .context("OL has no finalized EE account state")?;
    let expected_next_seq_no = target_update_seq_no
        .checked_add(1)
        .context("target update sequence overflow")?;
    if finalized_state.seq_no() != expected_next_seq_no {
        bail!(
            "target update sequence {target_update_seq_no} is not OL-finalized \
             (finalized next sequence is {})",
            finalized_state.seq_no()
        );
    }
    if finalized_state.inner_state().0 != expected_inner_state_root.0 {
        bail!("finalized OL EE state root does not match update sequence {target_update_seq_no}");
    }
    if finalized_state.next_inbox_msg_idx() != next_inbox_msg_idx {
        bail!(
            "finalized OL inbox cursor {} does not match update sequence {target_update_seq_no} \
             cursor {next_inbox_msg_idx}",
            finalized_state.next_inbox_msg_idx()
        );
    }

    let chain_status = client
        .chain_status()
        .await
        .context("loading finalized OL epoch commitment")?;
    let finalized_epoch = *chain_status.finalized();
    let finalized_slot = finalized_epoch.last_slot();
    let mut finalized_summaries = client
        .get_blocks_summaries(account_id, finalized_slot, finalized_slot)
        .await
        .context("loading the finalized OL anchor account summary")?;
    if finalized_summaries.len() != 1 {
        bail!(
            "expected one account summary for finalized OL slot {finalized_slot}, got {}",
            finalized_summaries.len()
        );
    }
    let finalized_summary = finalized_summaries
        .pop()
        .expect("summary count was validated above");
    let expected_finalized_commitment = finalized_epoch.to_block_commitment();
    if finalized_summary.block_commitment() != &expected_finalized_commitment {
        bail!(
            "OL returned account summary for block {}, expected finalized anchor \
             {expected_finalized_commitment}",
            finalized_summary.block_commitment()
        );
    }
    validate_anchor_inbox_cursor(next_inbox_msg_idx, finalized_summary.next_inbox_msg_idx())?;

    Ok(OlRecoveryTarget {
        last_exec_blkid: B256::from(decoded.new_tip_blkid().0),
        expected_inner_state_root: B256::from(expected_inner_state_root.0),
        finalized_epoch,
        previous_batch_block_hash,
        next_inbox_msg_idx,
        next_deposit_idx,
        pending_inputs,
        pending_fincls,
    })
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{BitcoinAmount, MsgPayload, SubjectId};
    use strata_codec::encode_to_vec;
    use strata_ee_acct_types::{DepositMsgData, DEPOSIT_MSG_TYPE};
    use strata_msg_fmt::{Msg, OwnedMsg};

    use super::{apply_queue_transition, parse_account_id, validate_anchor_inbox_cursor, *};

    fn deposit_message(destination: [u8; 32], value_sats: u64) -> MessageEntry {
        let body = encode_to_vec(&DepositMsgData::new(SubjectId::new(destination))).unwrap();
        let message = OwnedMsg::new(DEPOSIT_MSG_TYPE, body).unwrap();
        let payload =
            MsgPayload::from_bytes(BitcoinAmount::from_sat(value_sats), message.to_vec()).unwrap();
        MessageEntry::new(AccountId::new([9; 32]), 0, payload)
    }

    fn extra_data(processed_inputs: u32, processed_fincls: u32) -> UpdateExtraData {
        UpdateExtraData::new(
            Hash::from([1; 32]),
            Hash::from([2; 32]),
            processed_inputs,
            processed_fincls,
        )
    }

    #[test]
    fn parses_prefixed_account_id() {
        let account_id = parse_account_id(&format!("0x{}", "01".repeat(32))).unwrap();

        assert_eq!(account_id.to_string(), "01".repeat(32));
    }

    #[test]
    fn rejects_wrong_length_account_id() {
        let error = parse_account_id("01").unwrap_err();

        assert!(error.to_string().contains("parsing EE account ID"));
    }

    #[test]
    fn reconstructs_remaining_pending_deposits() {
        let mut state = EeAccountState::new(Hash::zero(), Hash::zero(), Vec::new(), Vec::new());
        let messages = [deposit_message([1; 32], 10), deposit_message([2; 32], 20)];

        apply_queue_transition(&mut state, &messages, &extra_data(1, 0), 0).unwrap();

        assert_eq!(state.pending_inputs().len(), 1);
        let PendingInputEntry::Deposit(remaining) = &state.pending_inputs()[0];
        assert_eq!(remaining.dest(), SubjectId::new([2; 32]));
        assert_eq!(remaining.value(), BitcoinAmount::from_sat(20));
    }

    #[test]
    fn drains_pending_deposits_across_updates() {
        let mut state = EeAccountState::new(Hash::zero(), Hash::zero(), Vec::new(), Vec::new());
        let messages = [deposit_message([1; 32], 10), deposit_message([2; 32], 20)];
        apply_queue_transition(&mut state, &messages, &extra_data(1, 0), 0).unwrap();

        apply_queue_transition(&mut state, &[], &extra_data(1, 0), 1).unwrap();

        assert!(state.pending_inputs().is_empty());
    }

    #[test]
    fn rejects_processed_input_underflow() {
        let mut state = EeAccountState::new(Hash::zero(), Hash::zero(), Vec::new(), Vec::new());

        let error = apply_queue_transition(&mut state, &[], &extra_data(1, 0), 0).unwrap_err();

        assert!(error.to_string().contains("only 0 are pending"));
    }

    #[test]
    fn rejects_forced_inclusions_without_a_producer() {
        let mut state = EeAccountState::new(Hash::zero(), Hash::zero(), Vec::new(), Vec::new());

        let error = apply_queue_transition(&mut state, &[], &extra_data(0, 1), 0).unwrap_err();

        assert!(error.to_string().contains("no forced-inclusion producer"));
    }

    #[test]
    fn accepts_cursor_at_finalized_anchor_inbox_tip() {
        validate_anchor_inbox_cursor(7, 7).unwrap();
    }

    #[test]
    fn rejects_cursor_before_finalized_anchor_inbox_tip() {
        let error = validate_anchor_inbox_cursor(7, 9).unwrap_err();

        assert!(error.to_string().contains(
            "finalized OL anchor contains inbox messages at or after recovered cursor 7"
        ));
    }

    #[test]
    fn rejects_cursor_beyond_finalized_anchor_inbox_tip() {
        let error = validate_anchor_inbox_cursor(9, 7).unwrap_err();

        assert!(error
            .to_string()
            .contains("finalized OL anchor inbox has 7 leaves, fewer than recovered cursor 9"));
    }
}
