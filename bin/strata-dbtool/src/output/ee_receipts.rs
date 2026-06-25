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

/// Acknowledgement payload for the `ee-delete-*-receipt` / `ee-delete-acct-proof`
/// commands.
///
/// `existed` always reflects the primary row (the chunk receipt or the acct
/// proof). `task_existed` is `Some` only for the chunk command, which also
/// deletes the companion PaaS task record under the same key so the chunk
/// re-proves; it is `None` for acct proofs, which have no companion task here.
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
    fn deleted_chunk_info_reports_task_existence() {
        let ack = DeletedEeReceiptInfo {
            address: "abcd".into(),
            kind: "chunk",
            existed: true,
            task_existed: Some(true),
        };
        let out = ack.format_porcelain();
        assert!(out.contains("existed: true"));
        assert!(out.contains("task_existed: true"));
    }
}
