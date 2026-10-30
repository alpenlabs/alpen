//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **argh** and delegates to command modules.

mod cli;
mod cmd;
mod db;
mod output;
mod utils;

use std::{path::Path, process::exit};

use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_store_sled::SledBackend;
use strata_db_types::backend::DatabaseBackend;
use strata_ol_canonical_chain_provider::{
    FinalizedOLChainProvider, OLCanonicalChainProvider, OLCanonicalChainProviderImpl,
};
use strata_storage::{create_node_storage, NodeStorage};
use tokio::runtime::Builder;
use tracing_subscriber::fmt::init;

use crate::{
    cli::{Cli, Command, NodeRole},
    cmd::{
        broadcaster::{get_broadcaster_summary, get_broadcaster_tx},
        checkpoint::{get_checkpoint, get_checkpoints_summary, get_epoch_summary},
        checkpoint_proof::{delete_checkpoint_proof, get_checkpoint_proof},
        client_state::get_client_state_update,
        l1::{get_l1_block, get_l1_summary},
        mmr::{get_mmr_leaf, get_mmr_summary},
        ol::{delete_ol_block, get_ol_block, get_ol_blocks_at_slot, get_ol_summary},
        ol_state::{get_ol_state, revert_ol_state},
        prover_task::{
            abandon_prover_task, abandon_prover_tasks, backfill_checkpoint_proof_task,
            backfill_prover_task_raw, delete_prover_task, get_prover_task,
            get_prover_tasks_summary, reset_prover_task,
        },
        syncinfo::get_syncinfo,
        terminal_header::backfill_terminal_headers,
        writer::{get_writer_payload, get_writer_summary},
    },
    db::open_database,
};

fn main() {
    init();

    let Cli { datadir, cmd } = argh::from_env();

    // Open the OL database once per command. Sled takes an exclusive directory lock,
    // so commands needing storage managers reuse that same backend.
    let result = match cmd {
        Command::GetOlBlock(args) => with_ol_db(&datadir, |db| get_ol_block(db, args)),
        Command::GetOlBlocksAtSlot(args) => {
            with_ol_db(&datadir, |db| get_ol_blocks_at_slot(db, args))
        }
        Command::GetOlSummary(args) => with_ol_db(&datadir, |db| get_ol_summary(db, args)),
        Command::DeleteOlBlock(args) => with_ol_db(&datadir, |db| delete_ol_block(db, args)),
        Command::GetMmrSummary(args) => with_ol_db(&datadir, |db| get_mmr_summary(db, args)),
        Command::GetMmrLeaf(args) => with_ol_db(&datadir, |db| get_mmr_leaf(db, args)),
        Command::GetL1Block(args) => with_ol_db(&datadir, |db| get_l1_block(db, args)),
        Command::GetL1Summary(args) => with_ol_db(&datadir, |db| get_l1_summary(db, args)),
        Command::GetWriterPayload(args) => with_ol_db(&datadir, |db| get_writer_payload(db, args)),
        Command::GetBroadcasterSummary(args) => with_ol_db(&datadir, |db| {
            get_broadcaster_summary(db.broadcast_db(), args)
        }),
        Command::GetBroadcasterTx(args) => {
            with_ol_db(&datadir, |db| get_broadcaster_tx(db.broadcast_db(), args))
        }
        Command::BackfillTerminalHeaders(args) => {
            with_ol_storage(&datadir, |storage| backfill_terminal_headers(storage, args))
        }
        Command::GetClientStateUpdate(args) => {
            with_ol_db(&datadir, |db| get_client_state_update(db, args))
        }
        Command::GetProverTask(args) => with_ol_db(&datadir, |db| get_prover_task(db, args)),
        Command::GetProverTasksSummary(args) => {
            with_ol_db(&datadir, |db| get_prover_tasks_summary(db, args))
        }
        Command::AbandonProverTask(args) => {
            with_ol_db(&datadir, |db| abandon_prover_task(db, args))
        }
        Command::AbandonProverTasks(args) => {
            with_ol_db(&datadir, |db| abandon_prover_tasks(db, args))
        }
        Command::ResetProverTask(args) => with_ol_db(&datadir, |db| reset_prover_task(db, args)),
        Command::DeleteProverTask(args) => with_ol_db(&datadir, |db| delete_prover_task(db, args)),
        Command::BackfillProverTaskRaw(args) => {
            with_ol_db(&datadir, |db| backfill_prover_task_raw(db, args))
        }
        command @ (Command::GetCheckpoint(_)
        | Command::GetCheckpointsSummary(_)
        | Command::GetEpochSummary(_)
        | Command::GetSyncinfo(_)
        | Command::GetCheckpointProof(_)
        | Command::DeleteCheckpointProof(_)
        | Command::BackfillCheckpointProofTask(_)
        | Command::GetWriterSummary(_)
        | Command::GetOLState(_)
        | Command::RevertOLState(_)) => {
            with_ol_storage(&datadir, |storage| run_with_node_role(storage, command))
        }
    };

    if let Err(e) = result {
        eprintln!("{e}");
        exit(1);
    }
}

/// Selects the canonical chain provider from the command's node role.
fn run_with_node_role(storage: &NodeStorage, command: Command) -> Result<(), DisplayedError> {
    let node_role = match &command {
        Command::GetCheckpoint(args) => args.node_role,
        Command::GetCheckpointsSummary(args) => args.node_role,
        Command::GetEpochSummary(args) => args.node_role,
        Command::GetSyncinfo(args) => args.node_role,
        Command::GetCheckpointProof(args) => args.node_role,
        Command::DeleteCheckpointProof(args) => args.node_role,
        Command::BackfillCheckpointProofTask(args) => args.node_role,
        Command::GetWriterSummary(args) => args.node_role,
        Command::GetOLState(args) => args.node_role,
        Command::RevertOLState(args) => args.node_role,
        _ => unreachable!("commands that don't need a node role are dispatched in main"),
    };
    match node_role {
        NodeRole::Sequencer => run_with_canonical_provider(
            storage.db().as_ref(),
            &OLCanonicalChainProviderImpl::new(storage),
            command,
        ),
        NodeRole::CheckpointSync => run_with_canonical_provider(
            storage.db().as_ref(),
            &FinalizedOLChainProvider::new(storage),
            command,
        ),
    }
}

fn run_with_canonical_provider(
    db: &impl DatabaseBackend,
    canonical_chain_provider: &impl OLCanonicalChainProvider,
    command: Command,
) -> Result<(), DisplayedError> {
    match command {
        Command::GetOLState(args) => get_ol_state(db, canonical_chain_provider, args),
        Command::RevertOLState(args) => revert_ol_state(db, canonical_chain_provider, args),
        Command::GetWriterSummary(args) => get_writer_summary(db, canonical_chain_provider, args),
        Command::GetCheckpoint(args) => get_checkpoint(db, canonical_chain_provider, args),
        Command::GetCheckpointsSummary(args) => {
            get_checkpoints_summary(db, canonical_chain_provider, args)
        }
        Command::GetEpochSummary(args) => get_epoch_summary(db, canonical_chain_provider, args),
        Command::GetSyncinfo(args) => get_syncinfo(db, canonical_chain_provider, args),
        Command::GetCheckpointProof(args) => {
            get_checkpoint_proof(db, canonical_chain_provider, args)
        }
        Command::DeleteCheckpointProof(args) => {
            delete_checkpoint_proof(db, canonical_chain_provider, args)
        }
        Command::BackfillCheckpointProofTask(args) => {
            backfill_checkpoint_proof_task(db, canonical_chain_provider, args)
        }
        _ => unreachable!("commands that don't need a node role are dispatched in main"),
    }
}

/// Opens the OL sled and constructs the storage managers needed by maintenance commands.
fn with_ol_storage<F, R>(datadir: &Path, f: F) -> R
where
    F: FnOnce(&NodeStorage) -> R,
{
    let rt = Builder::new_multi_thread()
        .enable_all()
        .build()
        .internal_error("Could not initialize dbtool Tokio runtime")
        .unwrap_or_else(|e: DisplayedError| {
            eprintln!("{e}");
            exit(1);
        });
    let db = open_database(datadir).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    let storage = create_node_storage(db, rt.handle().clone())
        .internal_error("Could not initialize OL storage managers")
        .unwrap_or_else(|e: DisplayedError| {
            eprintln!("{e}");
            exit(1);
        });
    f(&storage)
}

/// Opens the OL sled at `datadir` and runs `f` against it.
fn with_ol_db<F, R>(datadir: &Path, f: F) -> R
where
    F: FnOnce(&SledBackend) -> R,
{
    let db = open_database(datadir).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    f(db.as_ref())
}

#[cfg(test)]
mod tests {
    use argh::FromArgs;
    use strata_asm_checkpoint_types::{
        test_utils::create_test_checkpoint_payload, CheckpointPayload, CheckpointTip,
    };
    use strata_checkpoint_types::{CheckpointProofTask, EpochSummary};
    use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{ol_state::OLStateDatabase, prover_task::ProverTaskDatabase};
    use strata_identifiers::{
        Buf32, Buf64, EpochCommitment, L1BlockCommitment, OLBlockCommitment, RBuf32,
    };
    use strata_ol_chain_types_v1::{
        BlockFlagsV1, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLTxSegmentV1,
        SignedOLBlockHeaderV1,
    };
    use strata_ol_state_container::test_utils::create_test_genesis_container;
    use strata_storage::{test_runtime_handle, VersionedTaskStore};

    use super::*;

    struct CanonicalEpochFixture {
        storage: NodeStorage,
        fork: EpochCommitment,
        selected: EpochCommitment,
    }

    impl CanonicalEpochFixture {
        fn new() -> Self {
            let storage =
                create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap();
            let genesis = Self::store_block(&storage, None, true, 0);
            let pivot = Self::store_block(&storage, Some(&genesis), false, 1);
            let fork = Self::store_block(&storage, Some(&pivot), true, 2);
            let replacement = Self::store_block(&storage, Some(&pivot), false, 3);
            let selected = Self::store_block(&storage, Some(&replacement), true, 4);
            for header in [&genesis, &fork, &selected] {
                storage
                    .ol_checkpoint()
                    .insert_epoch_summary_blocking(EpochSummary::new(
                        header.epoch(),
                        header.compute_block_commitment(),
                        if header.slot() == 0 {
                            OLBlockCommitment::null()
                        } else {
                            genesis.compute_block_commitment()
                        },
                        L1BlockCommitment::new(100, Buf32::from([100; 32]).into()),
                        *header.state_root(),
                    ))
                    .unwrap();
            }
            storage
                .ol_block()
                .replace_canonical_suffix_from_blocking(
                    0,
                    vec![
                        genesis.compute_blkid(),
                        pivot.compute_blkid(),
                        fork.compute_blkid(),
                    ],
                )
                .unwrap();
            storage
                .ol_block()
                .replace_canonical_suffix_from_blocking(
                    2,
                    vec![replacement.compute_blkid(), selected.compute_blkid()],
                )
                .unwrap();
            Self {
                storage,
                fork: EpochCommitment::from_terminal(fork.epoch(), fork.compute_block_commitment()),
                selected: EpochCommitment::from_terminal(
                    selected.epoch(),
                    selected.compute_block_commitment(),
                ),
            }
        }

        fn store_block(
            storage: &NodeStorage,
            parent: Option<&OLBlockHeaderV1>,
            terminal: bool,
            timestamp: u64,
        ) -> OLBlockHeaderV1 {
            let state = create_test_genesis_container();
            let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![]).unwrap());
            let mut flags = BlockFlagsV1::zero();
            flags.set_is_terminal(terminal);
            let header = OLBlockHeaderV1::new(
                timestamp,
                flags,
                parent.map_or(0, |p| p.slot() + 1),
                parent.map_or(0, |p| p.epoch() + u32::from(p.is_terminal())),
                parent.map_or_else(|| Buf32::zero().into(), OLBlockHeaderV1::compute_blkid),
                body.compute_hash_commitment(),
                state.compute_state_root(),
                Buf32::zero(),
            );
            storage
                .ol_block()
                .put_block_data_blocking(OLBlockV1::new(
                    SignedOLBlockHeaderV1::new(header.clone(), Buf64::zero()),
                    body,
                ))
                .unwrap();
            storage
                .db()
                .ol_state_db()
                .put_toplevel_ol_state(header.compute_block_commitment(), state)
                .unwrap();
            header
        }

        fn observe(&self, commitment: EpochCommitment) {
            let block = L1BlockCommitment::new(101, Buf32::from([101; 32]).into());
            self.storage
                .l1()
                .extend_canonical_chain(block.blkid(), block.height())
                .unwrap();
            let sample = create_test_checkpoint_payload(commitment.epoch());
            let payload = CheckpointPayload::new(
                CheckpointTip::new(
                    commitment.epoch(),
                    block.height(),
                    commitment.to_block_commitment(),
                ),
                sample.sidecar().clone(),
                sample.proof().to_vec(),
            )
            .unwrap();
            self.storage
                .ol_checkpoint()
                .put_checkpoint_l1_observation_blocking(
                    commitment,
                    payload,
                    CheckpointL1Ref::new(block, RBuf32::from([1; 32]), RBuf32::from([2; 32])),
                )
                .unwrap();
        }

        fn finalize(&self, commitment: EpochCommitment) -> L1BlockCommitment {
            let block = L1BlockCommitment::new(102, Buf32::from([102; 32]).into());
            self.storage
                .l1()
                .extend_canonical_chain(block.blkid(), block.height())
                .unwrap();
            let checkpoint = L1Checkpoint::new(
                CheckpointTip::new(
                    commitment.epoch(),
                    block.height(),
                    commitment.to_block_commitment(),
                ),
                CheckpointL1Ref::new(block, RBuf32::from([3; 32]), RBuf32::from([4; 32])),
            );
            self.storage
                .client_state()
                .put_update_blocking(
                    &block,
                    ClientUpdateOutput::new_state(ClientState::new(
                        Some(checkpoint.clone()),
                        Some(checkpoint),
                    )),
                )
                .unwrap();
            block
        }

        fn backfill(&self, role: NodeRole) -> Result<(), DisplayedError> {
            let role = match role {
                NodeRole::Sequencer => "sequencer",
                NodeRole::CheckpointSync => "checkpoint-sync",
            };
            let cli = Cli::from_args(
                &["strata-dbtool"],
                &[
                    "backfill-checkpoint-proof-task",
                    "1",
                    "--node-role",
                    role,
                    "--force",
                ],
            )
            .unwrap();
            run_with_node_role(&self.storage, cli.cmd)
        }

        fn tasks(&self) -> Vec<EpochCommitment> {
            self.storage
                .db()
                .prover_task_db()
                .list_all_tasks()
                .unwrap()
                .iter()
                .map(|(key, _)| {
                    let (_, CheckpointProofTask(commitment)) =
                        VersionedTaskStore::decode_key(key).unwrap();
                    commitment
                })
                .collect()
        }
    }

    #[test]
    fn test_sequencer_command_uses_index_match_instead_of_first_candidate() {
        let fixture = CanonicalEpochFixture::new();
        assert_eq!(
            fixture
                .storage
                .ol_checkpoint()
                .get_epoch_commitments_at_blocking(1)
                .unwrap(),
            vec![fixture.fork, fixture.selected]
        );
        // The role must select the index even when finalized history points elsewhere.
        fixture.finalize(fixture.fork);
        fixture.backfill(NodeRole::Sequencer).unwrap();
        assert_eq!(fixture.tasks(), vec![fixture.selected]);
    }

    #[test]
    fn test_sequencer_command_does_not_fall_back_when_index_has_no_match() {
        let fixture = CanonicalEpochFixture::new();
        fixture.finalize(fixture.fork);
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(2, vec![])
            .unwrap();
        let error = fixture.backfill(NodeRole::Sequencer).unwrap_err();
        assert!(matches!(error, DisplayedError::UserError(_, _)));
        assert!(error
            .to_string()
            .contains("No canonical checkpoint commitment"));
        assert_eq!(fixture.tasks(), vec![]);
    }

    #[test]
    fn test_checkpoint_sync_command_uses_observed_finalized_history() {
        let fixture = CanonicalEpochFixture::new();
        fixture.observe(fixture.selected);
        let terminal = fixture
            .storage
            .ol_block()
            .get_ol_header_blocking(*fixture.selected.last_blkid())
            .unwrap()
            .unwrap();
        let next = CanonicalEpochFixture::store_block(&fixture.storage, Some(&terminal), true, 5);
        fixture.finalize(EpochCommitment::from_terminal(
            next.epoch(),
            next.compute_block_commitment(),
        ));
        // Finalized lookup must not depend on OL canonical rows or local summaries.
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(1, vec![])
            .unwrap();
        fixture
            .storage
            .ol_checkpoint()
            .del_epoch_summaries_from_epoch_blocking(1)
            .unwrap();
        fixture.backfill(NodeRole::CheckpointSync).unwrap();
        assert_eq!(fixture.tasks(), vec![fixture.selected]);
    }

    #[test]
    fn test_checkpoint_sync_command_preserves_provider_errors() {
        let fixture = CanonicalEpochFixture::new();
        let finalized_at = fixture.finalize(fixture.selected);
        fixture
            .storage
            .l1()
            .revert_canonical_chain(finalized_at.height() - 1)
            .unwrap();
        let error = fixture.backfill(NodeRole::CheckpointSync).unwrap_err();
        assert!(matches!(error, DisplayedError::InternalError(_, _)));
        assert!(error
            .to_string()
            .contains("OLFinalizedClientStateUnavailable"));
        assert_eq!(fixture.tasks(), vec![]);
    }

    #[test]
    fn test_checkpoint_sync_command_does_not_use_unfinalized_summaries() {
        let fixture = CanonicalEpochFixture::new();
        fixture.observe(fixture.selected);
        let error = fixture.backfill(NodeRole::CheckpointSync).unwrap_err();
        assert!(matches!(error, DisplayedError::UserError(_, _)));
        assert!(error
            .to_string()
            .contains("No canonical checkpoint commitment"));
        assert_eq!(fixture.tasks(), vec![]);
    }

    #[test]
    fn test_checkpoint_sync_command_reports_missing_finalized_observation() {
        let fixture = CanonicalEpochFixture::new();
        let terminal = fixture
            .storage
            .ol_block()
            .get_ol_header_blocking(*fixture.selected.last_blkid())
            .unwrap()
            .unwrap();
        let next = CanonicalEpochFixture::store_block(&fixture.storage, Some(&terminal), true, 5);
        fixture.finalize(EpochCommitment::from_terminal(
            next.epoch(),
            next.compute_block_commitment(),
        ));
        let error = fixture.backfill(NodeRole::CheckpointSync).unwrap_err();
        assert!(matches!(error, DisplayedError::InternalError(_, _)));
        assert!(error
            .to_string()
            .contains("OLFinalizedHistoryUnavailable { epoch: 1 }"));
        assert_eq!(fixture.tasks(), vec![]);
    }
}
