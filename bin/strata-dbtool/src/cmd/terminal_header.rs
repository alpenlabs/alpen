//! Backfill checkpoint terminal headers in pre-eager-persistence datadirs.

use argh::FromArgs;
use strata_checkpoint_types::reconstruct_terminal_header;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_ledger_types::IStateAccessor;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_storage::NodeStorage;

use crate::{
    cli::OutputFormat,
    output::{output, terminal_header::BackfillTerminalHeadersReport},
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backfill-terminal-headers")]
/// Backfill terminal headers from stored L1-observed checkpoints.
pub(crate) struct BackfillTerminalHeadersArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Backfills missing terminal-header records and reports all unavailable payloads.
pub(crate) fn backfill_terminal_headers(
    storage: &NodeStorage,
    args: BackfillTerminalHeadersArgs,
) -> Result<(), DisplayedError> {
    let report = build_terminal_header_backfill_report(storage)?;
    let missing_epochs = report.missing_observed_payload_epochs.clone();
    output(&report, args.output_format)?;

    if missing_epochs.is_empty() {
        Ok(())
    } else {
        Err(DisplayedError::UserError(
            "Terminal-header backfill incomplete; missing observed checkpoint payloads for epochs"
                .to_string(),
            Box::new(missing_epochs),
        ))
    }
}

fn build_terminal_header_backfill_report(
    storage: &NodeStorage,
) -> Result<BackfillTerminalHeadersReport, DisplayedError> {
    let mut report = BackfillTerminalHeadersReport::default();
    let Some(latest_epoch) = storage
        .ol_checkpoint()
        .get_last_summarized_epoch_blocking()
        .internal_error("Failed to get latest summarized OL epoch")?
    else {
        return Ok(report);
    };

    for epoch in 1..=latest_epoch {
        report.epochs_scanned += 1;

        let commitment = storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_blocking(epoch)
            .internal_error(format!(
                "Failed to get canonical OL epoch commitment at epoch {epoch}"
            ))?
            .ok_or_else(|| {
                DisplayedError::UserError(
                    "Missing canonical OL epoch commitment at applied epoch".to_string(),
                    Box::new(epoch),
                )
            })?;
        let summary = storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(commitment)
            .internal_error(format!("Failed to get OL epoch summary at epoch {epoch}"))?
            .ok_or_else(|| {
                DisplayedError::UserError(
                    "Missing OL epoch summary for canonical commitment".to_string(),
                    Box::new(commitment),
                )
            })?;
        let terminal = *summary.terminal();
        let terminal_blkid = *terminal.blkid();

        if storage
            .ol_block()
            .get_terminal_header_blocking(terminal_blkid)
            .internal_error(format!(
                "Failed to read terminal-header record at epoch {epoch}"
            ))?
            .is_some()
        {
            report.headers_skipped += 1;
            continue;
        }

        let Some(payload) = storage
            .ol_checkpoint()
            .get_checkpoint_l1_observed_payload_blocking(commitment)
            .internal_error(format!(
                "Failed to read L1-observed checkpoint payload at epoch {epoch}"
            ))?
        else {
            report.headers_not_backfilled += 1;
            report.missing_observed_payload_epochs.push(epoch);
            continue;
        };

        let header = reconstruct_terminal_header(
            payload.new_tip(),
            payload.sidecar().terminal_header_complement(),
            *summary.final_state(),
        )
        .user_error(format!(
            "Checkpoint terminal-header validation failed at epoch {epoch}"
        ))?;
        let reconstructed_blkid = header.compute_blkid();
        if reconstructed_blkid != terminal_blkid {
            return Err(DisplayedError::UserError(
                format!(
                    "Reconstructed terminal block ID does not match epoch summary at epoch {epoch}"
                ),
                Box::new((terminal_blkid, reconstructed_blkid)),
            ));
        }

        if let Some(state) = storage
            .ol_state()
            .get_toplevel_ol_state_blocking(terminal)
            .internal_error(format!("Failed to read terminal OL state at epoch {epoch}"))?
        {
            let stored_state_root = MemoryStateBaseLayer::new((*state).clone())
                .compute_state_root()
                .internal_error(format!(
                    "Failed to compute stored terminal OL state root at epoch {epoch}"
                ))?;
            if &stored_state_root != summary.final_state() {
                return Err(DisplayedError::UserError(
                    format!(
                        "Stored terminal OL state root does not match epoch summary at epoch {epoch}"
                    ),
                    Box::new((*summary.final_state(), stored_state_root)),
                ));
            }
        }

        storage
            .ol_block()
            .put_terminal_header_blocking(terminal_blkid, header)
            .internal_error(format!(
                "Failed to store terminal-header record at epoch {epoch}"
            ))?;
        report.headers_written += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use strata_asm_proto_checkpoint_types::{
        CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
    };
    use strata_checkpoint_types::EpochSummary;
    use strata_csm_types::CheckpointL1Ref;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{
        Buf32, Epoch, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
    };
    use strata_ol_chain_types::{BlockFlags, OLBlockHeader};
    use strata_ol_params::OLParams;
    use strata_ol_state_types::OLState;
    use strata_storage::create_node_storage;
    use tokio::runtime::{Handle, Runtime};

    use super::*;

    struct EpochFixture {
        commitment: strata_identifiers::EpochCommitment,
        terminal_blkid: OLBlockId,
        payload: CheckpointPayload,
        summary: EpochSummary,
    }

    fn test_runtime_handle() -> Handle {
        static RUNTIME: OnceLock<Runtime> = OnceLock::new();
        RUNTIME
            .get_or_init(|| Runtime::new().expect("create test Tokio runtime"))
            .handle()
            .clone()
    }

    fn storage() -> NodeStorage {
        create_node_storage(get_test_sled_backend(), test_runtime_handle())
            .expect("create test node storage")
    }

    fn epoch_fixture(epoch: Epoch, slot: u64, seed: u8) -> EpochFixture {
        let state_root = Buf32::from([seed; 32]);
        let complement = TerminalHeaderComplement::new(
            1_700_000_000 + u64::from(epoch),
            OLBlockId::from(Buf32::from([seed.wrapping_add(1); 32])),
            Buf32::from([seed.wrapping_add(2); 32]),
            Buf32::from([seed.wrapping_add(3); 32]),
        );
        let mut flags = BlockFlags::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeader::new(
            complement.timestamp(),
            flags,
            slot,
            epoch,
            *complement.parent_blkid(),
            *complement.body_root(),
            state_root,
            *complement.logs_root(),
        );
        let terminal = OLBlockCommitment::new(slot, header.compute_blkid());
        let tip = CheckpointTip::new(epoch, 100 + epoch, terminal);
        let sidecar = CheckpointSidecar::new(Vec::new(), Vec::new(), complement)
            .expect("create checkpoint sidecar");
        let payload =
            CheckpointPayload::new(tip, sidecar, Vec::new()).expect("create checkpoint payload");
        let summary = EpochSummary::new(
            epoch,
            terminal,
            OLBlockCommitment::null(),
            L1BlockCommitment::new(100 + epoch, L1BlockId::from(Buf32::from([seed; 32]))),
            state_root,
        );

        EpochFixture {
            commitment: summary.get_epoch_commitment(),
            terminal_blkid: header.compute_blkid(),
            payload,
            summary,
        }
    }

    fn insert_fixture(storage: &NodeStorage, fixture: &EpochFixture, observed: bool) {
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(fixture.summary)
            .expect("insert epoch summary");
        if observed {
            let l1_ref = CheckpointL1Ref::new(
                L1BlockCommitment::new(
                    200 + fixture.summary.epoch(),
                    L1BlockId::from(Buf32::from([0x55; 32])),
                ),
                [0x66; 32].into(),
                [0x77; 32].into(),
            );
            storage
                .ol_checkpoint()
                .put_checkpoint_l1_observation_blocking(
                    fixture.commitment,
                    fixture.payload.clone(),
                    l1_ref,
                )
                .expect("insert observed checkpoint payload");
        }
    }

    #[test]
    fn fresh_backfill_writes_all_headers() {
        let storage = storage();
        let fixtures = [epoch_fixture(1, 10, 1), epoch_fixture(2, 20, 2)];
        for fixture in &fixtures {
            insert_fixture(&storage, fixture, true);
        }

        let report = build_terminal_header_backfill_report(&storage).expect("backfill headers");

        assert_eq!(report.epochs_scanned, 2);
        assert_eq!(report.headers_written, 2);
        assert_eq!(report.headers_skipped, 0);
        assert_eq!(report.headers_not_backfilled, 0);
        for fixture in &fixtures {
            assert!(storage
                .ol_block()
                .get_terminal_header_blocking(fixture.terminal_blkid)
                .expect("read terminal header")
                .is_some());
        }
    }

    #[test]
    fn idempotent_rerun_writes_no_headers() {
        let storage = storage();
        let fixtures = [epoch_fixture(1, 10, 1), epoch_fixture(2, 20, 2)];
        for fixture in &fixtures {
            insert_fixture(&storage, fixture, true);
        }
        build_terminal_header_backfill_report(&storage).expect("initial backfill");

        let report = build_terminal_header_backfill_report(&storage).expect("repeat backfill");

        assert_eq!(report.epochs_scanned, 2);
        assert_eq!(report.headers_written, 0);
        assert_eq!(report.headers_skipped, 2);
        assert_eq!(report.headers_not_backfilled, 0);
    }

    #[test]
    fn missing_observed_payload_is_listed_after_other_epochs_are_processed() {
        let storage = storage();
        let first = epoch_fixture(1, 10, 1);
        let missing = epoch_fixture(2, 20, 2);
        let last = epoch_fixture(3, 30, 3);
        insert_fixture(&storage, &first, true);
        insert_fixture(&storage, &missing, false);
        insert_fixture(&storage, &last, true);

        let err = backfill_terminal_headers(
            &storage,
            BackfillTerminalHeadersArgs {
                output_format: OutputFormat::Porcelain,
            },
        )
        .expect_err("missing payload must fail the command");

        assert!(err.to_string().contains("epochs: [2]"));
        assert!(storage
            .ol_block()
            .get_terminal_header_blocking(first.terminal_blkid)
            .expect("read first terminal header")
            .is_some());
        assert!(storage
            .ol_block()
            .get_terminal_header_blocking(last.terminal_blkid)
            .expect("read last terminal header")
            .is_some());
        assert!(storage
            .ol_block()
            .get_terminal_header_blocking(missing.terminal_blkid)
            .expect("read missing terminal header")
            .is_none());
    }

    #[test]
    fn checkpoint_blkid_mismatch_is_hard_error_without_write() {
        let storage = storage();
        let mut fixture = epoch_fixture(1, 10, 1);
        let mismatched_terminal = OLBlockCommitment::new(
            fixture.summary.terminal().slot(),
            OLBlockId::from(Buf32::from([0xff; 32])),
        );
        let mismatched_tip = CheckpointTip::new(
            fixture.summary.epoch(),
            fixture.payload.new_tip().l1_height(),
            mismatched_terminal,
        );
        fixture.payload = CheckpointPayload::new(
            mismatched_tip,
            fixture.payload.sidecar().clone(),
            Vec::new(),
        )
        .expect("rebuild mismatched checkpoint payload");
        fixture.summary = EpochSummary::new(
            fixture.summary.epoch(),
            mismatched_terminal,
            *fixture.summary.prev_terminal(),
            *fixture.summary.new_l1(),
            *fixture.summary.final_state(),
        );
        fixture.commitment = fixture.summary.get_epoch_commitment();
        fixture.terminal_blkid = *mismatched_terminal.blkid();
        insert_fixture(&storage, &fixture, true);

        let err = build_terminal_header_backfill_report(&storage)
            .expect_err("mismatched block ID must fail");

        assert!(err
            .to_string()
            .contains("Checkpoint terminal-header validation failed at epoch 1"));
        assert!(storage
            .ol_block()
            .get_terminal_header_blocking(fixture.terminal_blkid)
            .expect("read mismatched terminal header")
            .is_none());
    }

    #[test]
    fn stored_state_root_mismatch_is_hard_error_without_write() {
        let storage = storage();
        let fixture = epoch_fixture(1, 10, 1);
        insert_fixture(&storage, &fixture, true);

        let stored_state = OLState::from_genesis_params(&OLParams::default())
            .expect("create stored terminal state");
        let stored_state_root = MemoryStateBaseLayer::new(stored_state.clone())
            .compute_state_root()
            .expect("compute stored terminal state root");
        assert_ne!(stored_state_root, *fixture.summary.final_state());
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(*fixture.summary.terminal(), stored_state)
            .expect("insert mismatched terminal state");

        let err = build_terminal_header_backfill_report(&storage)
            .expect_err("stored state-root mismatch must fail");

        assert!(err
            .to_string()
            .contains("Stored terminal OL state root does not match epoch summary at epoch 1"));
        assert!(storage
            .ol_block()
            .get_terminal_header_blocking(fixture.terminal_blkid)
            .expect("read terminal header after state-root mismatch")
            .is_none());
    }
}
