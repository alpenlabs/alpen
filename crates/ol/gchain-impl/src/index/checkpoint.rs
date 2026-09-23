//! Checkpoint links: DA reconstruction through the indexer layer, plus
//! recovering the per-update snark account records the DA diff collapses.

use std::collections::HashMap;

use strata_acct_types::{AccountId, AccountSerial};
use strata_msg_fmt::{Msg, MsgRef};
use strata_ol_chain_types_v1::{
    OLLog, OLLogType, SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID, SnarkAccountUpdateLogData,
};
use strata_ol_da_types_v1::OLDaSchemeV1;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{
    IndexerState, IndexerWrites, SnarkAcctStateUpdate, WriteTrackingState,
};
use strata_ol_state_types::{
    IAccountState, ISnarkAccountState, IStateAccessor, StateError, StateResult,
};
use strata_ol_stf_v1::apply_da_epoch;
use strata_snark_acct_types::Seqno;

use crate::errors::InvalidLinkError;
use crate::graph_types::OLCheckpointLink;
use crate::providers::{L1ManifestProvider, OLStateStore};
use crate::step::{PreState, StepError, assemble_checkpoint_inputs, exec_step_error};

pub(super) fn index_checkpoint_link<S: OLStateStore>(
    runtime_params: &OLRuntimeParams,
    manifest_provider: &impl L1ManifestProvider,
    pre_state: &PreState<'_, '_, S>,
    link: &OLCheckpointLink,
) -> Result<IndexerWrites, StepError> {
    let inputs = assemble_checkpoint_inputs(manifest_provider, pre_state, link)?;
    let ol_logs = link.payload().sidecar().ol_logs();

    // Each affected snark account's cursors as of the pre-state, so the
    // per-update records can be replayed from the logs afterwards.
    let pre_cursors = collect_pre_snark_account_cursors(pre_state, ol_logs)?;

    let mut state = IndexerState::new(WriteTrackingState::new_empty(pre_state));
    apply_da_epoch::<_, OLDaSchemeV1>(
        &mut state,
        &inputs.epoch_info,
        inputs.da_payload,
        &inputs.manifests,
        runtime_params,
    )
    .map_err(exec_step_error)?;

    // The DA diff collapses each account's updates into one; the logs are
    // what recover the per-update records.
    let recons = reconstruct_snark_acct_update_records(&state, ol_logs, &pre_cursors)?;
    verify_snark_seqno_invariant(&state, &recons.acct_cursors)?;

    let (_, mut writes) = state.into_parts();
    writes.set_snark_acct_state_updates(recons.updates);
    Ok(writes)
}

#[derive(Clone, Copy, Default)]
struct SnarkAccountCursor {
    seqno: Seqno,
    next_inbox_msg_idx: u64,
}

type SnarkAccountCursors = HashMap<AccountSerial, SnarkAccountCursor>;

struct ReconstructedSnarkData {
    /// Snark acct updates as they appear in OL logs.
    updates: Vec<SnarkAcctStateUpdate>,

    /// Cursors (next inbox idx and seqno) for each account after the logs.
    acct_cursors: SnarkAccountCursors,
}

/// Reads each affected snark account's pre-epoch cursor state.
///
/// The account set is inferred from `ol_logs`, which records the changes made
/// during the epoch.
fn collect_pre_snark_account_cursors(
    pre_state: &impl IStateAccessor,
    ol_logs: &[OLLog],
) -> Result<SnarkAccountCursors, StepError> {
    let mut pre_cursors = HashMap::new();
    for log in ol_logs {
        let serial = log.account_serial();
        if log_msg(log)?.ty() != SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID
            || pre_cursors.contains_key(&serial)
        {
            continue;
        }
        pre_cursors.insert(serial, get_snark_acct_cursor(pre_state, serial)?);
    }
    Ok(pre_cursors)
}

/// Reads an account's cursor from the state, defaulting to zero for an
/// account that doesn't exist yet.
fn get_snark_acct_cursor(
    state: &impl IStateAccessor,
    serial: AccountSerial,
) -> Result<SnarkAccountCursor, StateError> {
    let Some(account_id) = state.find_account_id_by_serial(serial)? else {
        return Ok(SnarkAccountCursor::default());
    };
    match get_snark_acct(state, account_id) {
        Ok(snacct) => Ok(SnarkAccountCursor {
            seqno: snacct.seqno(),
            next_inbox_msg_idx: snacct.next_inbox_msg_idx(),
        }),
        Err(StateError::MissingAccount(_)) => Ok(SnarkAccountCursor::default()),
        Err(e) => Err(e),
    }
}

/// Reconstructs per-update snark index records from the epoch's OL logs.
fn reconstruct_snark_acct_update_records(
    state: &impl IStateAccessor,
    ol_logs: &[OLLog],
    pre_cursors: &SnarkAccountCursors,
) -> Result<ReconstructedSnarkData, StepError> {
    let mut acct_cursors: SnarkAccountCursors = HashMap::new();
    let mut updates = Vec::with_capacity(ol_logs.len());
    // Index of each account's last update, so we can stamp the recoverable
    // post-epoch root onto it after the walk.
    let mut last_record_idxs: HashMap<AccountId, usize> = HashMap::new();

    for log in ol_logs {
        let serial = log.account_serial();
        let msg = log_msg(log)?;
        if msg.ty() != SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID {
            continue;
        }
        let log_data = SnarkAccountUpdateLogData::try_decode_log(&msg)
            .map_err(InvalidLinkError::SnarkUpdateLogDecode)?;
        let account_id = find_account_id(state, serial)?;

        let cur = acct_cursors
            .entry(serial)
            .or_insert_with(|| pre_cursors.get(&serial).copied().unwrap_or_default());
        let prev_next_read_idx = cur.next_inbox_msg_idx;
        cur.seqno = cur.seqno.incr();
        cur.next_inbox_msg_idx = log_data.new_msg_idx;

        // Intermediate per-update roots are not in checkpoint logs; the
        // terminal update is patched below with the recoverable post-epoch
        // root.
        last_record_idxs.insert(account_id, updates.len());
        updates.push(SnarkAcctStateUpdate::new(
            account_id,
            None,
            prev_next_read_idx,
            log_data.new_msg_idx,
            cur.seqno,
        ));
    }

    // The post-DA state holds each account's final inner state root, which is
    // the terminal update's post-state root.  Earlier updates stay `None`.
    for (account_id, idx) in last_record_idxs {
        let root = get_snark_acct(state, account_id)?.inner_state_root();
        updates[idx].set_state(Some(root));
    }

    Ok(ReconstructedSnarkData {
        updates,
        acct_cursors,
    })
}

/// Checks that the seqno derived by walking the logs matches the seqno the DA
/// diff put in the post-state, which is the logs and the diff agreeing on how
/// many updates the epoch had.
fn verify_snark_seqno_invariant(
    state: &impl IStateAccessor,
    acct_cursors: &SnarkAccountCursors,
) -> Result<(), StepError> {
    for (&serial, cursor) in acct_cursors {
        let account_id = find_account_id(state, serial)?;
        let diff = get_snark_acct(state, account_id)?.seqno();
        if diff != cursor.seqno {
            return Err(InvalidLinkError::SnarkSeqnoMismatch {
                account_id,
                logs: cursor.seqno,
                diff,
            }
            .into());
        }
    }
    Ok(())
}

fn log_msg(log: &OLLog) -> Result<MsgRef<'_>, InvalidLinkError> {
    Ok(MsgRef::try_from(log.payload())?)
}

/// A log naming a serial with no account is a log the diff doesn't account
/// for, so the link is invalid rather than the lookup having failed.
fn find_account_id(
    state: &impl IStateAccessor,
    serial: AccountSerial,
) -> Result<AccountId, StepError> {
    Ok(state
        .find_account_id_by_serial(serial)?
        .ok_or(InvalidLinkError::UnknownAccountSerial(serial))?)
}

fn get_snark_acct<S: IStateAccessor>(
    state: &S,
    account_id: AccountId,
) -> StateResult<&<S::AccountState as IAccountState>::SnarkAccountState> {
    let Some(acct) = state.get_account_state(account_id)? else {
        return Err(StateError::MissingAccount(account_id));
    };
    acct.as_snark_account()
}
