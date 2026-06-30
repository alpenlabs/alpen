//! Output types for EE rollback maintenance commands.

use serde::Serialize;

use super::{
    helpers::{porcelain_bool, porcelain_field},
    prover_task::StatusInfo,
    traits::Formattable,
};

#[derive(Serialize)]
pub(crate) struct EeRevertBatchesReport {
    pub(crate) dry_run: bool,
    pub(crate) from_batch_idx: u64,
    pub(crate) revert_to_batch_idx: u64,
    pub(crate) latest_batch_idx_before: u64,
    pub(crate) first_reverted_block_height: u64,
    pub(crate) first_reverted_block_hash: String,
    pub(crate) local_accepted_frontier: AcceptedFrontierInfo,
    pub(crate) account_state_rollback: AccountStateRollbackInfo,
    pub(crate) warnings: Vec<String>,
    pub(crate) blocked: bool,
    pub(crate) block_reason: Option<String>,
    pub(crate) affected_block_summary: AffectedBlockSummary,
    pub(crate) affected_batches: Vec<AffectedBatchInfo>,
    pub(crate) affected_chunks: Vec<AffectedChunkInfo>,
    pub(crate) affected_blocks: Vec<AffectedBlockInfo>,
    pub(crate) orphan_notes: Vec<String>,
    pub(crate) tx_export_path: Option<String>,
    pub(crate) mutation: MutationInfo,
}

#[derive(Serialize)]
pub(crate) struct AffectedBlockSummary {
    pub(crate) total_count: usize,
    pub(crate) reverted_batch: BlockRangeSummary,
    pub(crate) unbatched_suffix: BlockRangeSummary,
}

#[derive(Serialize)]
pub(crate) struct BlockRangeSummary {
    pub(crate) count: usize,
    pub(crate) first_blocknum: Option<u64>,
    pub(crate) last_blocknum: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct AcceptedFrontierInfo {
    pub(crate) source: &'static str,
    pub(crate) last_exec_block_hash: Option<String>,
    pub(crate) best_epoch: Option<u32>,
    pub(crate) accepted_batch_idx: Option<u64>,
    pub(crate) crosses_accepted_frontier: bool,
    pub(crate) caveat: &'static str,
}

#[derive(Serialize)]
pub(crate) struct AccountStateRollbackInfo {
    pub(crate) required: bool,
    pub(crate) target_epoch: Option<u32>,
    pub(crate) target_exec_block_hash: Option<String>,
    pub(crate) performed: bool,
}

#[derive(Serialize)]
pub(crate) struct AffectedBatchInfo {
    pub(crate) idx: u64,
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) update_seq_no: Option<u64>,
    pub(crate) prev_block: String,
    pub(crate) last_block: String,
    pub(crate) last_blocknum: u64,
    pub(crate) block_count: usize,
    pub(crate) chunk_count: usize,
    pub(crate) acct_task: ProverArtifactInfo,
    pub(crate) acct_proof_exists: bool,
    pub(crate) acct_proof_deleted: bool,
}

#[derive(Serialize)]
pub(crate) struct AffectedChunkInfo {
    pub(crate) batch_idx: u64,
    pub(crate) idx: Option<u64>,
    pub(crate) id: String,
    pub(crate) status: Option<String>,
    pub(crate) prev_block: String,
    pub(crate) last_block: String,
    pub(crate) last_blocknum: Option<u64>,
    pub(crate) block_count: Option<usize>,
    pub(crate) task: ProverArtifactInfo,
    pub(crate) receipt_exists: bool,
    pub(crate) receipt_deleted: bool,
}

#[derive(Serialize)]
pub(crate) struct ProverArtifactInfo {
    pub(crate) key_hex: String,
    pub(crate) existed: bool,
    pub(crate) deleted: bool,
    pub(crate) status: Option<StatusInfo>,
}

#[derive(Serialize)]
pub(crate) struct AffectedBlockInfo {
    pub(crate) blocknum: u64,
    pub(crate) hash: String,
    pub(crate) parent_hash: String,
    pub(crate) batch_idx: Option<u64>,
    pub(crate) in_reverted_batch: bool,
    pub(crate) delete_planned: bool,
    pub(crate) deleted: bool,
    pub(crate) tx_count: usize,
    pub(crate) transactions: Vec<BlockTransactionInfo>,
}

#[derive(Serialize)]
pub(crate) struct BlockTransactionInfo {
    pub(crate) index: usize,
    pub(crate) hash: String,
    #[serde(skip_serializing)]
    pub(crate) raw_tx_hex: Option<String>,
}

#[derive(Serialize, Default)]
pub(crate) struct MutationInfo {
    pub(crate) force: bool,
    pub(crate) tx_export_written: bool,
    pub(crate) batch_rows_reverted: usize,
    pub(crate) exec_blocks_deleted: usize,
    pub(crate) chunk_tasks_deleted: usize,
    pub(crate) chunk_receipts_deleted: usize,
    pub(crate) acct_tasks_deleted: usize,
    pub(crate) acct_proofs_deleted: usize,
}

impl Formattable for EeRevertBatchesReport {
    fn format_porcelain(&self) -> String {
        let mut out = vec![
            porcelain_field("dry_run", porcelain_bool(self.dry_run)),
            porcelain_field("from_batch_idx", self.from_batch_idx),
            porcelain_field("revert_to_batch_idx", self.revert_to_batch_idx),
            porcelain_field("latest_batch_idx_before", self.latest_batch_idx_before),
            porcelain_field(
                "first_reverted_block_height",
                self.first_reverted_block_height,
            ),
            porcelain_field("first_reverted_block_hash", &self.first_reverted_block_hash),
            porcelain_field("blocked", porcelain_bool(self.blocked)),
        ];

        if let Some(reason) = &self.block_reason {
            out.push(porcelain_field("block_reason", reason));
        }

        out.push(porcelain_field(
            "local_accepted_frontier.source",
            self.local_accepted_frontier.source,
        ));
        if let Some(hash) = &self.local_accepted_frontier.last_exec_block_hash {
            out.push(porcelain_field(
                "local_accepted_frontier.last_exec_block_hash",
                hash,
            ));
        }
        if let Some(idx) = self.local_accepted_frontier.accepted_batch_idx {
            out.push(porcelain_field(
                "local_accepted_frontier.accepted_batch_idx",
                idx,
            ));
        }
        if let Some(epoch) = self.local_accepted_frontier.best_epoch {
            out.push(porcelain_field("local_accepted_frontier.best_epoch", epoch));
        }
        out.push(porcelain_field(
            "local_accepted_frontier.crosses_accepted_frontier",
            porcelain_bool(self.local_accepted_frontier.crosses_accepted_frontier),
        ));
        out.push(porcelain_field(
            "local_accepted_frontier.caveat",
            self.local_accepted_frontier.caveat,
        ));
        out.push(porcelain_field(
            "account_state_rollback.required",
            porcelain_bool(self.account_state_rollback.required),
        ));
        if let Some(epoch) = self.account_state_rollback.target_epoch {
            out.push(porcelain_field(
                "account_state_rollback.target_epoch",
                epoch,
            ));
        }
        if let Some(hash) = &self.account_state_rollback.target_exec_block_hash {
            out.push(porcelain_field(
                "account_state_rollback.target_exec_block_hash",
                hash,
            ));
        }
        out.push(porcelain_field(
            "account_state_rollback.performed",
            porcelain_bool(self.account_state_rollback.performed),
        ));

        for (i, warning) in self.warnings.iter().enumerate() {
            out.push(porcelain_field(&format!("warnings[{i}]"), warning));
        }

        out.extend(self.affected_block_summary.format_porcelain());

        for (i, batch) in self.affected_batches.iter().enumerate() {
            out.extend(batch.format_porcelain(i));
        }

        for (i, chunk) in self.affected_chunks.iter().enumerate() {
            out.extend(chunk.format_porcelain(i));
        }

        for (i, block) in self.affected_blocks.iter().enumerate() {
            out.extend(block.format_porcelain(i));
        }

        for (i, note) in self.orphan_notes.iter().enumerate() {
            out.push(porcelain_field(&format!("orphan_notes[{i}]"), note));
        }

        if let Some(path) = &self.tx_export_path {
            out.push(porcelain_field("tx_export_path", path));
        }

        out.extend(self.mutation.format_porcelain());
        out.join("\n")
    }
}

impl AffectedBlockSummary {
    fn format_porcelain(&self) -> Vec<String> {
        let mut out = vec![porcelain_field(
            "affected_block_summary.total_count",
            self.total_count,
        )];
        out.extend(
            self.reverted_batch
                .format_porcelain("affected_block_summary.reverted_batch"),
        );
        out.extend(
            self.unbatched_suffix
                .format_porcelain("affected_block_summary.unbatched_suffix"),
        );
        out
    }
}

impl BlockRangeSummary {
    fn format_porcelain(&self, prefix: &str) -> Vec<String> {
        let mut out = vec![porcelain_field(&format!("{prefix}.count"), self.count)];
        if let Some(blocknum) = self.first_blocknum {
            out.push(porcelain_field(
                &format!("{prefix}.first_blocknum"),
                blocknum,
            ));
        }
        if let Some(blocknum) = self.last_blocknum {
            out.push(porcelain_field(
                &format!("{prefix}.last_blocknum"),
                blocknum,
            ));
        }
        out
    }
}

impl AffectedBatchInfo {
    fn format_porcelain(&self, i: usize) -> Vec<String> {
        let prefix = format!("affected_batches[{i}]");
        let mut out = vec![
            porcelain_field(&format!("{prefix}.idx"), self.idx),
            porcelain_field(&format!("{prefix}.id"), &self.id),
            porcelain_field(&format!("{prefix}.status"), &self.status),
            porcelain_field(&format!("{prefix}.prev_block"), &self.prev_block),
            porcelain_field(&format!("{prefix}.last_block"), &self.last_block),
            porcelain_field(&format!("{prefix}.last_blocknum"), self.last_blocknum),
            porcelain_field(&format!("{prefix}.block_count"), self.block_count),
            porcelain_field(&format!("{prefix}.chunk_count"), self.chunk_count),
            porcelain_field(
                &format!("{prefix}.acct_proof_exists"),
                porcelain_bool(self.acct_proof_exists),
            ),
            porcelain_field(
                &format!("{prefix}.acct_proof_deleted"),
                porcelain_bool(self.acct_proof_deleted),
            ),
        ];
        if let Some(seq_no) = self.update_seq_no {
            out.push(porcelain_field(&format!("{prefix}.update_seq_no"), seq_no));
        }
        out.extend(
            self.acct_task
                .format_porcelain(&format!("{prefix}.acct_task")),
        );
        out
    }
}

impl AffectedChunkInfo {
    fn format_porcelain(&self, i: usize) -> Vec<String> {
        let prefix = format!("affected_chunks[{i}]");
        let mut out = vec![
            porcelain_field(&format!("{prefix}.batch_idx"), self.batch_idx),
            porcelain_field(&format!("{prefix}.id"), &self.id),
            porcelain_field(&format!("{prefix}.prev_block"), &self.prev_block),
            porcelain_field(&format!("{prefix}.last_block"), &self.last_block),
            porcelain_field(
                &format!("{prefix}.receipt_exists"),
                porcelain_bool(self.receipt_exists),
            ),
            porcelain_field(
                &format!("{prefix}.receipt_deleted"),
                porcelain_bool(self.receipt_deleted),
            ),
        ];
        if let Some(idx) = self.idx {
            out.push(porcelain_field(&format!("{prefix}.idx"), idx));
        }
        if let Some(status) = &self.status {
            out.push(porcelain_field(&format!("{prefix}.status"), status));
        }
        if let Some(last_blocknum) = self.last_blocknum {
            out.push(porcelain_field(
                &format!("{prefix}.last_blocknum"),
                last_blocknum,
            ));
        }
        if let Some(block_count) = self.block_count {
            out.push(porcelain_field(
                &format!("{prefix}.block_count"),
                block_count,
            ));
        }
        out.extend(self.task.format_porcelain(&format!("{prefix}.task")));
        out
    }
}

impl ProverArtifactInfo {
    fn format_porcelain(&self, prefix: &str) -> Vec<String> {
        let mut out = vec![
            porcelain_field(&format!("{prefix}.key_hex"), &self.key_hex),
            porcelain_field(&format!("{prefix}.existed"), porcelain_bool(self.existed)),
            porcelain_field(&format!("{prefix}.deleted"), porcelain_bool(self.deleted)),
        ];
        if let Some(status) = &self.status {
            out.push(porcelain_field(&format!("{prefix}.status"), status.name));
            if let Some(retry_count) = status.retry_count {
                out.push(porcelain_field(
                    &format!("{prefix}.status.retry_count"),
                    retry_count,
                ));
            }
            if let Some(error) = &status.error {
                out.push(porcelain_field(&format!("{prefix}.status.error"), error));
            }
        }
        out
    }
}

impl AffectedBlockInfo {
    fn format_porcelain(&self, i: usize) -> Vec<String> {
        let prefix = format!("affected_blocks[{i}]");
        let mut out = vec![
            porcelain_field(&format!("{prefix}.blocknum"), self.blocknum),
            porcelain_field(&format!("{prefix}.hash"), &self.hash),
            porcelain_field(&format!("{prefix}.parent_hash"), &self.parent_hash),
            porcelain_field(
                &format!("{prefix}.in_reverted_batch"),
                porcelain_bool(self.in_reverted_batch),
            ),
            porcelain_field(
                &format!("{prefix}.delete_planned"),
                porcelain_bool(self.delete_planned),
            ),
            porcelain_field(&format!("{prefix}.deleted"), porcelain_bool(self.deleted)),
            porcelain_field(&format!("{prefix}.tx_count"), self.tx_count),
        ];
        if let Some(batch_idx) = self.batch_idx {
            out.push(porcelain_field(&format!("{prefix}.batch_idx"), batch_idx));
        }
        for (j, tx) in self.transactions.iter().enumerate() {
            out.push(porcelain_field(
                &format!("{prefix}.transactions[{j}].index"),
                tx.index,
            ));
            out.push(porcelain_field(
                &format!("{prefix}.transactions[{j}].hash"),
                &tx.hash,
            ));
        }
        out
    }
}

impl MutationInfo {
    fn format_porcelain(&self) -> Vec<String> {
        vec![
            porcelain_field("mutation.force", porcelain_bool(self.force)),
            porcelain_field(
                "mutation.tx_export_written",
                porcelain_bool(self.tx_export_written),
            ),
            porcelain_field("mutation.batch_rows_reverted", self.batch_rows_reverted),
            porcelain_field("mutation.exec_blocks_deleted", self.exec_blocks_deleted),
            porcelain_field("mutation.chunk_tasks_deleted", self.chunk_tasks_deleted),
            porcelain_field(
                "mutation.chunk_receipts_deleted",
                self.chunk_receipts_deleted,
            ),
            porcelain_field("mutation.acct_tasks_deleted", self.acct_tasks_deleted),
            porcelain_field("mutation.acct_proofs_deleted", self.acct_proofs_deleted),
        ]
    }
}
