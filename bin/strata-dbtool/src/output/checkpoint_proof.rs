//! Output structs for the checkpoint-proof admin commands.

use serde::Serialize;
use strata_identifiers::{Epoch, OLBlockId};
use zkaleido::ProofReceiptWithMetadata;

use super::{helpers::porcelain_field, traits::Formattable};

/// Per-receipt detail emitted by `get-checkpoint-proof`.
#[derive(Serialize)]
pub(crate) struct CheckpointProofInfo {
    pub(crate) epoch: Epoch,
    pub(crate) terminal_blkid: OLBlockId,
    pub(crate) zkvm: String,
    pub(crate) proof_type: String,
    pub(crate) program_id_hex: String,
    pub(crate) program_version: String,
    pub(crate) proof_len: usize,
    pub(crate) public_values_len: usize,
}

impl CheckpointProofInfo {
    pub(crate) fn from_receipt(
        epoch: Epoch,
        terminal_blkid: OLBlockId,
        receipt: &ProofReceiptWithMetadata,
    ) -> Self {
        let metadata = receipt.metadata();
        Self {
            epoch,
            terminal_blkid,
            zkvm: format!("{:?}", metadata.zkvm()),
            proof_type: format!("{:?}", metadata.proof_type()),
            program_id_hex: hex::encode(metadata.program_id().0),
            program_version: metadata.version().to_string(),
            proof_len: receipt.receipt().proof().as_bytes().len(),
            public_values_len: receipt.receipt().public_values().as_bytes().len(),
        }
    }
}

impl Formattable for CheckpointProofInfo {
    fn format_porcelain(&self) -> String {
        let mut out = Vec::new();
        out.push(porcelain_field("epoch", self.epoch));
        out.push(porcelain_field(
            "terminal_blkid",
            format!("{:?}", self.terminal_blkid),
        ));
        out.push(porcelain_field("zkvm", &self.zkvm));
        out.push(porcelain_field("proof_type", &self.proof_type));
        out.push(porcelain_field("program_id_hex", &self.program_id_hex));
        out.push(porcelain_field("program_version", &self.program_version));
        out.push(porcelain_field("proof_len", self.proof_len));
        out.push(porcelain_field("public_values_len", self.public_values_len));
        out.join("\n")
    }
}

/// Acknowledgement payload for `delete-checkpoint-proof`.
#[derive(Serialize)]
pub(crate) struct DeletedCheckpointProofInfo {
    pub(crate) epoch: Epoch,
    pub(crate) terminal_blkid: OLBlockId,
    pub(crate) existed: bool,
}

impl Formattable for DeletedCheckpointProofInfo {
    fn format_porcelain(&self) -> String {
        [
            porcelain_field("epoch", self.epoch),
            porcelain_field("terminal_blkid", format!("{:?}", self.terminal_blkid)),
            porcelain_field("existed", self.existed),
        ]
        .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use zkaleido::{
        ProgramId, Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, ProofType,
        PublicValues, ZkVm,
    };

    use super::*;

    fn fake_receipt() -> ProofReceiptWithMetadata {
        let metadata = ProofMetadata::new(
            ZkVm::Native,
            ProgramId([7u8; 32]),
            "0.1".to_string(),
            ProofType::Groth16,
        );
        let receipt = ProofReceipt::new(Proof::new(vec![1, 2, 3]), PublicValues::new(vec![9, 8]));
        ProofReceiptWithMetadata::new(receipt, metadata)
    }

    #[test]
    fn checkpoint_proof_info_records_metadata_and_lengths() {
        let receipt = fake_receipt();
        let info = CheckpointProofInfo::from_receipt(7u32, OLBlockId::default(), &receipt);

        assert_eq!(info.epoch, 7);
        assert_eq!(info.zkvm, "Native");
        assert_eq!(info.proof_type, "Groth16");
        assert_eq!(
            info.program_id_hex,
            "0707070707070707070707070707070707070707070707070707070707070707"
        );
        assert_eq!(info.program_version, "0.1");
        assert_eq!(info.proof_len, 3);
        assert_eq!(info.public_values_len, 2);
    }

    #[test]
    fn deleted_info_porcelain_is_stable() {
        let ack = DeletedCheckpointProofInfo {
            epoch: 12u32,
            terminal_blkid: OLBlockId::default(),
            existed: true,
        };
        let out = ack.format_porcelain();
        assert!(out.contains("epoch: 12"));
        assert!(out.contains("existed: true"));
    }
}
