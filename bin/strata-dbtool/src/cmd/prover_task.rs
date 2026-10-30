//! Admin commands operating on the OL prover task store.
//!
//! These talk directly to [`strata_db_types::prover_task::ProverTaskDatabase`]
//! so they can manipulate records without going through the running
//! prover service — by design, the node must be offline.
//!
//! Every mutating verb follows the `revert-ol-state` UX: without
//! `-f/--force` the command is a dry run (prints what *would* happen
//! and a `Use --force to execute these changes.` hint); with `--force`
//! the mutation actually lands.

use argh::FromArgs;
use strata_checkpoint_types::CheckpointProofTask;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::{
    backend::DatabaseBackend, ol_state::OLStateDatabase, prover_task::ProverTaskDatabase,
};
use strata_identifiers::Epoch;
use strata_paas::{TaskRecordData, TaskStatus};
use strata_storage::VersionedTaskStore;

use crate::{
    cli::OutputFormat,
    cmd::{
        checkpoint::get_canonical_epoch_commitment_at,
        prover_task_common::{parse_task_key, print_force_hint, StatusFilter, ABANDONED_REASON},
    },
    output::{
        output,
        prover_task::{ProverTaskInfo, ProverTasksSummaryInfo},
    },
};

/// Fetch a single prover task record by its hex-encoded key.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-prover-task")]
pub(crate) struct GetProverTaskArgs {
    /// hex-encoded task key (as stored by `ProverTaskDatabase`)
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Summarize prover tasks by status, with a bounded slice of entries.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-prover-tasks-summary")]
pub(crate) struct GetProverTasksSummaryArgs {
    /// status filter: all (default), pending, proving, completed,
    /// transient-failure, permanent-failure, unfinished, terminal
    #[argh(option, default = "StatusFilter::All")]
    pub(crate) status: StatusFilter,

    /// max number of matching entries to include in the output (default 20)
    #[argh(option, default = "20")]
    pub(crate) limit: usize,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Fetch a single prover task record by hex-encoded key.
pub(crate) fn get_prover_task(
    db: &impl DatabaseBackend,
    args: GetProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let record = db
        .prover_task_db()
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    let info = ProverTaskInfo::from_record(&key, &record);
    output(&info, args.output_format)
}

/// Summarize prover task store contents.
pub(crate) fn get_prover_tasks_summary(
    db: &impl DatabaseBackend,
    args: GetProverTasksSummaryArgs,
) -> Result<(), DisplayedError> {
    let task_db = db.prover_task_db();

    let all = task_db
        .list_all_tasks()
        .internal_error("Failed to list prover tasks")?;

    let mut pending = 0usize;
    let mut proving = 0usize;
    let mut completed = 0usize;
    let mut blocked = 0usize;
    let mut transient_failure = 0usize;
    let mut permanent_failure = 0usize;
    let mut matched = 0usize;
    let mut entries: Vec<ProverTaskInfo> = Vec::new();

    for (key, record) in &all {
        match record.status() {
            TaskStatus::Pending => pending += 1,
            TaskStatus::Proving { .. } => proving += 1,
            TaskStatus::Completed => completed += 1,
            TaskStatus::Blocked { .. } => blocked += 1,
            TaskStatus::TransientFailure { .. } => transient_failure += 1,
            TaskStatus::PermanentFailure { .. } => permanent_failure += 1,
        }
        if args.status.matches(record.status()) {
            matched += 1;
            if entries.len() < args.limit {
                entries.push(ProverTaskInfo::from_record(key, record));
            }
        }
    }

    let summary = ProverTasksSummaryInfo {
        total: all.len(),
        pending,
        proving,
        completed,
        blocked,
        transient_failure,
        permanent_failure,
        matched,
        returned: entries.len(),
        entries,
    };

    output(&summary, args.output_format)
}

/// Mark a single prover task as `PermanentFailure { error: "abandoned via dbtool" }`.
///
/// Leaves the record in the DB for audit; recovery will not respawn it.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "abandon-prover-task")]
pub(crate) struct AbandonProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Bulk-abandon every Pending/Proving prover task.
///
/// Use case: after a crash or operator-induced restart, prevent stuck
/// in-progress tasks from being respawned by the recovery scanner.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "abandon-prover-tasks")]
pub(crate) struct AbandonProverTasksArgs {
    /// only consider Pending/Proving tasks (currently the only supported
    /// selector — kept explicit so future selectors can be added)
    #[argh(switch)]
    pub(crate) all_unfinished: bool,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Reset a prover task to `Pending` and clear its retry-after timestamp.
///
/// Use case: force a fresh prove attempt (drops accumulated retry count).
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "reset-prover-task")]
pub(crate) struct ResetProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Hard-delete a prover task record.
///
/// Prefer `abandon-prover-task` unless you really want the row gone.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "delete-prover-task")]
pub(crate) struct DeleteProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Abandon a single task by flipping its status to `PermanentFailure`.
pub(crate) fn abandon_prover_task(
    db: &impl DatabaseBackend,
    args: AbandonProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let task_db = db.prover_task_db();

    let mut record = task_db
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    if record.status().is_terminal() {
        return Err(DisplayedError::UserError(
            "Task is already in a terminal state".to_string(),
            Box::new(args.key_hex),
        ));
    }

    if !args.force {
        println!("would abandon: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    record.set_status(TaskStatus::PermanentFailure {
        error: ABANDONED_REASON.to_string(),
    });
    task_db
        .put_task(key, record)
        .internal_error("Failed to persist abandoned task")?;

    println!("abandoned: {}", args.key_hex);
    Ok(())
}

/// Bulk-abandon every Pending/Proving task.
pub(crate) fn abandon_prover_tasks(
    db: &impl DatabaseBackend,
    args: AbandonProverTasksArgs,
) -> Result<(), DisplayedError> {
    if !args.all_unfinished {
        return Err(DisplayedError::UserError(
            "--all-unfinished is the only currently supported selector".to_string(),
            Box::new(()),
        ));
    }

    let task_db = db.prover_task_db();
    let unfinished = task_db
        .list_unfinished()
        .internal_error("Failed to list unfinished prover tasks")?;

    let mut abandoned = 0usize;
    for (key, mut record) in unfinished {
        let key_hex = hex::encode(&key);
        if args.force {
            record.set_status(TaskStatus::PermanentFailure {
                error: ABANDONED_REASON.to_string(),
            });
            task_db
                .put_task(key, record)
                .internal_error("Failed to persist abandoned task")?;
            println!("abandoned: {key_hex}");
        } else {
            println!("would abandon: {key_hex}");
        }
        abandoned += 1;
    }

    let verb = if args.force {
        "abandoned"
    } else {
        "would abandon"
    };
    println!("{verb} {abandoned} task(s)");
    if !args.force {
        print_force_hint();
    }
    Ok(())
}

/// Reset a task to `Pending` and clear its retry-after timestamp.
pub(crate) fn reset_prover_task(
    db: &impl DatabaseBackend,
    args: ResetProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let task_db = db.prover_task_db();

    let mut record = task_db
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    if !args.force {
        println!("would reset: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    record.set_status(TaskStatus::Pending);
    record.set_retry_after_secs(None);
    task_db
        .put_task(key, record)
        .internal_error("Failed to persist reset task")?;

    println!("reset: {}", args.key_hex);
    Ok(())
}

/// Hard-delete a task row.
pub(crate) fn delete_prover_task(
    db: &impl DatabaseBackend,
    args: DeleteProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let task_db = db.prover_task_db();

    // Resolve existence up front so the dry run can surface a clear
    // error rather than silently "previewing" a no-op delete.
    let exists = task_db
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .is_some();
    if !exists {
        return Err(DisplayedError::UserError(
            "No prover task found for key".to_string(),
            Box::new(args.key_hex),
        ));
    }

    if !args.force {
        println!("would delete: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    task_db
        .delete_task(key)
        .internal_error("Failed to delete prover task")?;

    println!("deleted: {}", args.key_hex);
    Ok(())
}

/// Queue a fresh `Pending` checkpoint-proof task for an epoch.
///
/// Resolves the canonical commitment and uses its terminal OL state's current
/// spec to construct the task key via [`VersionedTaskStore`]. A prover service
/// for that spec can recover the task on startup. Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backfill-checkpoint-proof-task")]
pub(crate) struct BackfillCheckpointProofTaskArgs {
    /// checkpoint epoch
    #[argh(positional)]
    pub(crate) epoch: Epoch,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Insert a `Pending` task record under a raw hex-encoded key.
///
/// Escape hatch for proof kinds without a typed helper. Caller is
/// responsible for matching the key format the host expects.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backfill-prover-task-raw")]
pub(crate) struct BackfillProverTaskRawArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Insert a Pending checkpoint-proof task for the canonical commitment at
/// the given epoch.
pub(crate) fn backfill_checkpoint_proof_task(
    db: &impl DatabaseBackend,
    args: BackfillCheckpointProofTaskArgs,
) -> Result<(), DisplayedError> {
    let commitment = get_canonical_epoch_commitment_at(db, args.epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No canonical checkpoint commitment at epoch".to_string(),
            Box::new(args.epoch),
        )
    })?;
    let terminal = commitment.to_block_commitment();
    // The current spec identifies the completed epoch.
    let spec = db
        .ol_state_db()
        .get_toplevel_ol_state(terminal)
        .internal_error(format!(
            "Failed to read terminal OL state at {terminal:?} for epoch {}",
            args.epoch
        ))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No terminal OL state for epoch {}", args.epoch),
                Box::new(terminal),
            )
        })?
        .cur_spec();
    let key = VersionedTaskStore::encode_key(spec, CheckpointProofTask(commitment));
    let key_hex = hex::encode(&key);

    if !args.force {
        println!(
            "would backfill checkpoint proof task: {key_hex} (epoch {})",
            args.epoch
        );
        print_force_hint();
        return Ok(());
    }

    let record = TaskRecordData::new(TaskStatus::Pending);
    db.prover_task_db()
        .insert_task(key, record)
        .internal_error("Failed to insert prover task")?;

    println!(
        "backfilled checkpoint proof task: {key_hex} (epoch {})",
        args.epoch
    );
    Ok(())
}

/// Insert a Pending task under a caller-provided raw key.
pub(crate) fn backfill_prover_task_raw(
    db: &impl DatabaseBackend,
    args: BackfillProverTaskRawArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;

    if !args.force {
        println!("would backfill prover task: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    let record = TaskRecordData::new(TaskStatus::Pending);
    db.prover_task_db()
        .insert_task(key, record)
        .internal_error("Failed to insert prover task")?;

    println!("backfilled prover task: {}", args.key_hex);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ciborium::{from_reader, into_writer, value::Value};
    use sled::Config as SledConfig;
    use strata_checkpoint_types::EpochSummary;
    use strata_db_store_sled::{test_utils::get_test_sled_backend, SledBackend, SledDbConfig};
    use strata_db_types::ol_checkpoint::OLCheckpointDatabase;
    use strata_identifiers::{Buf32, EpochCommitment, L1BlockCommitment, OLBlockCommitment};
    use strata_ol_state_container::{
        test_utils::{
            create_test_container_with_staged, create_test_genesis_container, v0_container,
        },
        OLStateContainer,
    };
    use strata_ol_state_types::OLSpecId;
    use strata_ol_state_types_v1::test_utils::create_test_genesis_state;
    use typed_sled::SledDb;

    use super::*;

    fn seed_epoch(db: &impl DatabaseBackend, state: OLStateContainer) -> EpochCommitment {
        let terminal = OLBlockCommitment::new(10, Buf32::from([1; 32]).into());
        let summary = EpochSummary::new(
            1,
            terminal,
            OLBlockCommitment::new(9, Buf32::from([2; 32]).into()),
            L1BlockCommitment::new(100, Buf32::from([3; 32]).into()),
            state.compute_state_root(),
        );
        db.ol_state_db()
            .put_toplevel_ol_state(terminal, state)
            .unwrap();
        db.ol_checkpoint_db().insert_epoch_summary(summary).unwrap();
        summary.get_epoch_commitment()
    }

    fn backfill(db: &impl DatabaseBackend, force: bool) -> Result<(), DisplayedError> {
        backfill_checkpoint_proof_task(db, BackfillCheckpointProofTaskArgs { epoch: 1, force })
    }

    #[test]
    fn backfill_uses_terminal_current_spec() {
        for (state, expected_spec) in [
            (v0_container(create_test_genesis_state()), OLSpecId::V0),
            (create_test_genesis_container(), OLSpecId::V1),
            (create_test_container_with_staged(0), OLSpecId::V1),
            (create_test_container_with_staged(2), OLSpecId::V1),
        ] {
            let db = get_test_sled_backend();
            let commitment = seed_epoch(db.as_ref(), state);
            // A newer snapshot must not determine this epoch's spec.
            db.ol_state_db()
                .put_toplevel_ol_state(
                    OLBlockCommitment::new(11, Buf32::from([4; 32]).into()),
                    create_test_genesis_container(),
                )
                .unwrap();

            backfill(db.as_ref(), true).unwrap();

            let tasks = db.prover_task_db().list_all_tasks().unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(
                VersionedTaskStore::decode_key(&tasks[0].0),
                Some((expected_spec, CheckpointProofTask(commitment)))
            );
            assert_eq!(tasks[0].1.status(), &TaskStatus::Pending);
        }
    }

    #[test]
    fn backfill_missing_terminal_state_does_not_use_another_snapshot() {
        for force in [false, true] {
            let db = get_test_sled_backend();
            let commitment = seed_epoch(db.as_ref(), create_test_genesis_container());
            db.ol_state_db()
                .del_toplevel_ol_state(commitment.to_block_commitment())
                .unwrap();
            for (slot, block_id) in [(9, [2; 32]), (10, [4; 32]), (11, [5; 32])] {
                db.ol_state_db()
                    .put_toplevel_ol_state(
                        OLBlockCommitment::new(slot, Buf32::from(block_id).into()),
                        create_test_genesis_container(),
                    )
                    .unwrap();
            }

            let error = backfill(db.as_ref(), force).unwrap_err();

            assert!(matches!(error, DisplayedError::UserError(_, _)));
            assert!(error.to_string().contains("terminal OL state"));
            assert_eq!(db.prover_task_db().list_all_tasks().unwrap().len(), 0);
        }
    }

    #[test]
    fn backfill_unreadable_terminal_spec_does_not_insert_a_task() {
        let raw_db = SledConfig::new().temporary(true).open().unwrap();
        let db = SledBackend::new(
            Arc::new(SledDb::new(raw_db.clone()).unwrap()),
            SledDbConfig::test(),
        )
        .unwrap();
        seed_epoch(&db, create_test_genesis_container());

        // Inject an unsupported current spec into the actual stored CBOR snapshot.
        // Typed state constructors cannot produce this invalid state.
        let tree = raw_db.open_tree("OLStateSchema").unwrap();
        let (key, encoded) = tree.first().unwrap().unwrap();
        let mut value: Value = from_reader(encoded.as_ref()).unwrap();
        let (_, current_spec) = value
            .as_map_mut()
            .unwrap()
            .iter_mut()
            .find(|(field, _)| field.as_text() == Some("cur_spec_version"))
            .unwrap();
        *current_spec = Value::Integer(99.into());
        let mut invalid = Vec::new();
        into_writer(&value, &mut invalid).unwrap();
        tree.insert(key, invalid).unwrap();

        for force in [false, true] {
            let error = backfill(&db, force).unwrap_err();
            assert!(matches!(error, DisplayedError::InternalError(_, _)));
            assert!(error.to_string().contains("terminal OL state"));
            assert_eq!(db.prover_task_db().list_all_tasks().unwrap().len(), 0);
        }
    }

    #[test]
    fn backfill_without_canonical_epoch_does_not_insert_a_task() {
        let db = get_test_sled_backend();
        for force in [false, true] {
            let error = backfill(db.as_ref(), force).unwrap_err();
            assert!(error
                .to_string()
                .contains("No canonical checkpoint commitment"));
            assert_eq!(db.prover_task_db().list_all_tasks().unwrap().len(), 0);
        }
    }

    #[test]
    fn backfill_dry_run_does_not_insert_a_task() {
        let db = get_test_sled_backend();
        seed_epoch(db.as_ref(), create_test_genesis_container());

        backfill(db.as_ref(), false).unwrap();

        assert_eq!(db.prover_task_db().list_all_tasks().unwrap().len(), 0);
    }

    #[test]
    fn backfill_preserves_existing_task_even_with_force() {
        let db = get_test_sled_backend();
        let commitment = seed_epoch(db.as_ref(), create_test_genesis_container());
        let key = VersionedTaskStore::encode_key(OLSpecId::V1, CheckpointProofTask(commitment));
        let mut record = TaskRecordData::new(TaskStatus::Completed);
        record.set_metadata(Some(vec![1, 2, 3]));
        db.prover_task_db()
            .insert_task(key.clone(), record.clone())
            .unwrap();

        backfill(db.as_ref(), false).unwrap();
        let error = backfill(db.as_ref(), true).unwrap_err();

        assert!(error.to_string().contains("EntryAlreadyExists"));
        let tasks = db.prover_task_db().list_all_tasks().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].0, key);
        assert_eq!(tasks[0].1.status(), record.status());
        assert_eq!(tasks[0].1.metadata(), record.metadata());
        assert_eq!(tasks[0].1.updated_at_secs(), record.updated_at_secs());
        assert_eq!(tasks[0].1.retry_after_secs(), record.retry_after_secs());
    }
}
