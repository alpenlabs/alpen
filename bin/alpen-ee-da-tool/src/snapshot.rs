//! Replay snapshot loading, writing, and export assembly.

use std::{fs, path::Path};

use alpen_ee_da_state_replay::{ReplayPreStateSnapshot, ReplaySummary};
use alpen_ee_da_types::DaBlob;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

/// Loads a replay pre-state snapshot JSON file.
pub(crate) fn load_snapshot(path: &Path) -> Result<ReplayPreStateSnapshot, DisplayedError> {
    let contents = fs::read_to_string(path)
        .user_error(format!("failed to read snapshot {}", path.display()))?;
    serde_json::from_str(&contents)
        .user_error(format!("failed to parse snapshot {}", path.display()))
}

/// Writes a replay pre-state snapshot JSON file.
pub(crate) fn write_snapshot(
    path: &Path,
    snapshot: &ReplayPreStateSnapshot,
) -> Result<(), DisplayedError> {
    let contents = serde_json::to_string_pretty(snapshot)
        .internal_error("failed to serialize replay snapshot")?;
    fs::write(path, contents).user_error(format!("failed to write snapshot {}", path.display()))
}

/// Builds the post-run snapshot for `--export-snapshot`.
pub(crate) fn build_export_snapshot(
    input_snapshot: Option<&ReplayPreStateSnapshot>,
    blobs: &[DaBlob],
    replay_summary: &ReplaySummary,
) -> Result<ReplayPreStateSnapshot, DisplayedError> {
    if replay_summary.applied().is_none() && input_snapshot.is_none() {
        return Err(DisplayedError::UserError(
            "cannot export replay snapshot".to_string(),
            Box::new(SnapshotExportError::NoReplayInput),
        ));
    }

    let next_update_seq_no = match replay_summary.applied() {
        Some(applied) => applied.last_update_seq_no().checked_add(1).ok_or_else(|| {
            DisplayedError::InternalError(
                "cannot export snapshot after update_seq_no overflow".to_string(),
                Box::new(SnapshotExportError::UpdateSeqNoOverflow),
            )
        })?,
        None => input_snapshot
            .map(ReplayPreStateSnapshot::next_update_seq_no)
            .unwrap_or(0),
    };
    let last_applied_block_num = replay_summary
        .applied()
        .map(|applied| applied.last_block_num())
        .or_else(|| input_snapshot.map(ReplayPreStateSnapshot::last_applied_block_num))
        .unwrap_or(0);
    let mut bytecodes = input_snapshot
        .map(|snapshot| snapshot.bytecodes().clone())
        .unwrap_or_default();
    for blob in blobs {
        bytecodes.extend(
            blob.state_diff
                .deployed_bytecodes
                .iter()
                .map(|(code_hash, bytecode)| (*code_hash, bytecode.clone())),
        );
    }

    Ok(ReplayPreStateSnapshot::new(
        replay_summary.final_state_root(),
        next_update_seq_no,
        last_applied_block_num,
        replay_summary.final_state_seed().clone(),
        bytecodes,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotExportError {
    NoReplayInput,
    UpdateSeqNoOverflow,
}

#[cfg(test)]
mod tests {
    use alpen_ee_da_state_replay::replay_blobs_from_genesis;
    use alpen_ee_da_types::{DaBlob, EvmHeaderSummary};
    use strata_cli_common::errors::DisplayedError;

    use super::build_export_snapshot;

    #[test]
    fn build_export_snapshot_advances_applied_anchor() {
        let blobs = vec![test_blob(0, 100), test_blob(1, 101)];
        let replay_summary =
            replay_blobs_from_genesis("dev", &blobs).expect("genesis replay must succeed");

        let snapshot = build_export_snapshot(None, &blobs, &replay_summary)
            .expect("snapshot export must succeed");

        assert_eq!(
            snapshot.expected_state_root(),
            replay_summary.final_state_root()
        );
        assert_eq!(snapshot.next_update_seq_no(), 2);
        assert_eq!(snapshot.last_applied_block_num(), 101);
    }

    #[test]
    fn build_export_snapshot_rejects_empty_genesis_replay() {
        let replay_summary =
            replay_blobs_from_genesis("dev", &[]).expect("empty genesis replay must succeed");

        let error = build_export_snapshot(None, &[], &replay_summary)
            .expect_err("empty genesis replay must not export a snapshot");

        assert!(matches!(error, DisplayedError::UserError(_, _)));
        assert!(error.to_string().contains("cannot export replay snapshot"));
    }

    fn test_blob(update_seq_no: u64, block_num: u64) -> DaBlob {
        let gas_used = block_num % 1_000;
        DaBlob {
            update_seq_no,
            evm_header: EvmHeaderSummary {
                block_num,
                timestamp: block_num,
                base_fee: block_num,
                gas_used,
                gas_limit: gas_used.saturating_add(1),
            },
            state_diff: Default::default(),
        }
    }
}
