//! Output structs for the EE receipt admin commands.
//!
//! Chunk receipts and acct proofs share the same `ProofReceiptWithMetadata`
//! shape, so the inspection output is identical to OL's
//! `get-checkpoint-proof`. Only the addressing differs: chunk receipts
//! are keyed by an opaque task key (the chunk prover's task encoding),
//! and acct proofs are keyed by [`alpen_ee_common::BatchId`].

use serde::Serialize;
use zkaleido::ProofReceiptWithMetadata;

use super::{helpers::porcelain_field, traits::Formattable};

/// Per-receipt detail for the EE inspection commands. Mirrors the OL
/// `CheckpointProofInfo` shape but carries the EE-side identifier
/// (`address`) rather than an epoch + terminal blkid.
#[derive(Serialize)]
pub(crate) struct EeReceiptInfo {
    /// Human-readable identifier — chunk task key (hex) or batch id
    /// (`prev_block_hex:last_block_hex`), depending on the kind.
    pub(crate) address: String,
    pub(crate) kind: &'static str,
    pub(crate) zkvm: String,
    pub(crate) proof_type: String,
    pub(crate) program_id_hex: String,
    pub(crate) program_version: String,
    pub(crate) proof_len: usize,
    pub(crate) public_values_len: usize,
}

impl EeReceiptInfo {
    pub(crate) fn from_receipt(
        address: String,
        kind: &'static str,
        receipt: &ProofReceiptWithMetadata,
    ) -> Self {
        let metadata = receipt.metadata();
        Self {
            address,
            kind,
            zkvm: format!("{:?}", metadata.zkvm()),
            proof_type: format!("{:?}", metadata.proof_type()),
            program_id_hex: hex::encode(metadata.program_id().0),
            program_version: metadata.version().to_string(),
            proof_len: receipt.receipt().proof().as_bytes().len(),
            public_values_len: receipt.receipt().public_values().as_bytes().len(),
        }
    }
}

impl Formattable for EeReceiptInfo {
    fn format_porcelain(&self) -> String {
        [
            porcelain_field("address", &self.address),
            porcelain_field("kind", self.kind),
            porcelain_field("zkvm", &self.zkvm),
            porcelain_field("proof_type", &self.proof_type),
            porcelain_field("program_id_hex", &self.program_id_hex),
            porcelain_field("program_version", &self.program_version),
            porcelain_field("proof_len", self.proof_len),
            porcelain_field("public_values_len", self.public_values_len),
        ]
        .join("\n")
    }
}

/// Coupled invalidation report for `ee-delete-chunk-receipt`.
///
/// Deleting a chunk receipt invalidates the chunk's lifecycle state and any
/// acct proof derived from it. This report exposes both the discovered state
/// and the mutations that were actually applied.
#[derive(Serialize)]
pub(crate) struct EeChunkReproofInfo {
    pub(crate) address: String,
    pub(crate) kind: &'static str,
    pub(crate) dry_run: bool,
    pub(crate) existed: bool,
    pub(crate) task_existed: bool,
    pub(crate) chunk_id: String,
    pub(crate) chunk_status: &'static str,
    pub(crate) batch_id: String,
    pub(crate) batch_status: &'static str,
    pub(crate) acct_proof_existed: bool,
    pub(crate) acct_task_existed: bool,
    pub(crate) mutation: EeChunkReproofMutation,
}

#[derive(Default, Serialize)]
pub(crate) struct EeChunkReproofMutation {
    pub(crate) receipt_deleted: bool,
    pub(crate) chunk_task_deleted: bool,
    pub(crate) chunk_status_reset: bool,
    pub(crate) acct_proof_deleted: bool,
    pub(crate) acct_task_deleted: bool,
    pub(crate) batch_status_reset: bool,
}

impl Formattable for EeChunkReproofInfo {
    fn format_porcelain(&self) -> String {
        [
            porcelain_field("address", &self.address),
            porcelain_field("kind", self.kind),
            porcelain_field("dry_run", self.dry_run),
            porcelain_field("existed", self.existed),
            porcelain_field("task_existed", self.task_existed),
            porcelain_field("chunk_id", &self.chunk_id),
            porcelain_field("chunk_status", self.chunk_status),
            porcelain_field("batch_id", &self.batch_id),
            porcelain_field("batch_status", self.batch_status),
            porcelain_field("acct_proof_existed", self.acct_proof_existed),
            porcelain_field("acct_task_existed", self.acct_task_existed),
            porcelain_field("receipt_deleted", self.mutation.receipt_deleted),
            porcelain_field("chunk_task_deleted", self.mutation.chunk_task_deleted),
            porcelain_field("chunk_status_reset", self.mutation.chunk_status_reset),
            porcelain_field("acct_proof_deleted", self.mutation.acct_proof_deleted),
            porcelain_field("acct_task_deleted", self.mutation.acct_task_deleted),
            porcelain_field("batch_status_reset", self.mutation.batch_status_reset),
        ]
        .join("\n")
    }
}

/// Acknowledgement payload for the standalone `ee-delete-acct-proof` command.
///
/// `existed` reflects the acct proof row. `task_existed` remains optional for
/// output compatibility and is unset by this command.
#[derive(Serialize)]
pub(crate) struct DeletedEeReceiptInfo {
    pub(crate) address: String,
    pub(crate) kind: &'static str,
    pub(crate) existed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) task_existed: Option<bool>,
}

impl Formattable for DeletedEeReceiptInfo {
    fn format_porcelain(&self) -> String {
        let mut fields = vec![
            porcelain_field("address", &self.address),
            porcelain_field("kind", self.kind),
            porcelain_field("existed", self.existed),
        ];
        if let Some(task_existed) = self.task_existed {
            fields.push(porcelain_field("task_existed", task_existed));
        }
        fields.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use zkaleido::{ProgramId, Proof, ProofMetadata, ProofReceipt, ProofType, PublicValues, ZkVm};

    use super::*;

    fn fake_receipt() -> ProofReceiptWithMetadata {
        let metadata = ProofMetadata::new(
            ZkVm::Native,
            ProgramId([3u8; 32]),
            "0.2".to_string(),
            ProofType::Groth16,
        );
        let receipt = ProofReceipt::new(Proof::new(vec![1, 2]), PublicValues::new(vec![7]));
        ProofReceiptWithMetadata::new(receipt, metadata)
    }

    #[test]
    fn ee_receipt_info_records_kind_and_metadata() {
        let receipt = fake_receipt();
        let info = EeReceiptInfo::from_receipt("deadbeef".into(), "chunk", &receipt);

        assert_eq!(info.address, "deadbeef");
        assert_eq!(info.kind, "chunk");
        assert_eq!(info.zkvm, "Native");
        assert_eq!(info.proof_type, "Groth16");
        assert_eq!(info.proof_len, 2);
        assert_eq!(info.public_values_len, 1);

        let out = info.format_porcelain();
        assert!(out.contains("address: deadbeef"));
        assert!(out.contains("kind: chunk"));
    }

    #[test]
    fn deleted_info_porcelain_is_stable() {
        let ack = DeletedEeReceiptInfo {
            address: "abcd".into(),
            kind: "acct",
            existed: false,
            task_existed: None,
        };
        let out = ack.format_porcelain();
        assert!(out.contains("address: abcd"));
        assert!(out.contains("kind: acct"));
        assert!(out.contains("existed: false"));
        // No companion task for acct proofs, so the line is omitted.
        assert!(!out.contains("task_existed"));
    }

    #[test]
    fn chunk_reproof_info_reports_coupled_mutations() {
        let ack = EeChunkReproofInfo {
            address: "abcd".into(),
            kind: "chunk",
            dry_run: false,
            existed: true,
            task_existed: true,
            chunk_id: "chunk".into(),
            chunk_status: "proof_ready",
            batch_id: "batch".into(),
            batch_status: "proof_ready",
            acct_proof_existed: true,
            acct_task_existed: true,
            mutation: EeChunkReproofMutation {
                receipt_deleted: true,
                chunk_task_deleted: true,
                chunk_status_reset: true,
                acct_proof_deleted: true,
                acct_task_deleted: true,
                batch_status_reset: true,
            },
        };
        let out = ack.format_porcelain();
        assert!(out.contains("existed: true"));
        assert!(out.contains("task_existed: true"));
        assert!(out.contains("chunk_status_reset: true"));
        assert!(out.contains("acct_proof_deleted: true"));
        assert!(out.contains("batch_status_reset: true"));
    }
}
