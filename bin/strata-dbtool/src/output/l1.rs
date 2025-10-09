//! L1 block formatting implementations

use strata_asm_types::{L1Tx, ProtocolOperation};
use strata_identifiers::L1BlockId;

use super::{checkpoint::format_signed_checkpoint, helpers::porcelain_field, traits::Formattable};

/// Transaction information with computed IDs
#[derive(serde::Serialize)]
pub(crate) struct TransactionInfo {
    pub index: usize,
    pub txid: String,
    pub wtxid: String,
    pub protocol_ops_count: usize,
}

/// L1 block information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct L1BlockInfo<'a> {
    pub block_id: &'a L1BlockId,
    pub transactions: &'a [L1Tx],
    pub height: u64,
    pub transaction_infos: Vec<TransactionInfo>,
}

/// L1 summary information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct L1SummaryInfo {
    pub tip_height: u64,
    pub tip_block_id: String,
    pub from_height: u64,
    pub from_block_id: String,
    pub expected_block_count: u64,
    pub all_manifests_present: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_blocks: Vec<MissingBlockInfo>,
}

/// Information about missing blocks
#[derive(serde::Serialize)]
pub(crate) struct MissingBlockInfo {
    pub height: u64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
}

impl<'a> Formattable for L1BlockInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field("block_id", format!("{:?}", self.block_id)));
        output.push(porcelain_field("height", self.height));
        output.push(porcelain_field(
            "transaction_count",
            self.transactions.len(),
        ));

        // Add transaction information
        for (index, tx_info) in self.transaction_infos.iter().enumerate() {
            let tx_prefix = format!("tx_{index}");
            output.push(porcelain_field(&format!("{tx_prefix}.txid"), &tx_info.txid));
            output.push(porcelain_field(
                &format!("{tx_prefix}.wtxid"),
                &tx_info.wtxid,
            ));
            output.push(porcelain_field(
                &format!("{tx_prefix}.protocol_ops_count"),
                tx_info.protocol_ops_count,
            ));

            // Add protocol operation details
            if let Some(tx) = self.transactions.get(index) {
                for (op_index, op) in tx.protocol_ops().iter().enumerate() {
                    let op_prefix = format!("{tx_prefix}.protocol_op_{op_index}");
                    match op {
                        ProtocolOperation::Checkpoint(signed_checkpoint) => {
                            let prefix = format!("{op_prefix}.checkpoint");
                            output.extend(format_signed_checkpoint(signed_checkpoint, &prefix));
                        }
                        ProtocolOperation::DaCommitment(da_commitment) => {
                            output.push(porcelain_field(
                                &format!("{op_prefix}.da_commitment"),
                                format!("{da_commitment:?}"),
                            ));
                        }
                        ProtocolOperation::Deposit(deposit_info) => {
                            output.push(porcelain_field(
                                &format!("{op_prefix}.deposit"),
                                format!("{deposit_info:?}"),
                            ));
                        }
                        ProtocolOperation::DepositRequest(deposit_request) => {
                            output.push(porcelain_field(
                                &format!("{op_prefix}.deposit_request"),
                                format!("{deposit_request:?}"),
                            ));
                        }
                        ProtocolOperation::WithdrawalFulfillment(withdrawal_fulfillment) => {
                            output.push(porcelain_field(
                                &format!("{op_prefix}.withdrawal_fulfillment"),
                                format!("{withdrawal_fulfillment:?}"),
                            ));
                        }
                        ProtocolOperation::DepositSpent(deposit_spent) => {
                            output.push(porcelain_field(
                                &format!("{op_prefix}.deposit_spent"),
                                format!("{deposit_spent:?}"),
                            ));
                        }
                    }
                }
            }
        }

        output.join("\n")
    }
}

impl Formattable for L1SummaryInfo {
    fn format_porcelain(&self) -> String {
        let mut output = vec![
            porcelain_field("tip_height", self.tip_height),
            porcelain_field("tip_block_id", &self.tip_block_id),
            porcelain_field("from_height", self.from_height),
            porcelain_field("from_block_id", &self.from_block_id),
            porcelain_field("expected_block_count", self.expected_block_count),
            porcelain_field(
                "all_manifests_present",
                super::helpers::porcelain_bool(self.all_manifests_present),
            ),
        ];

        // Add missing block information if any
        for missing_block in &self.missing_blocks {
            let prefix = format!("missing_block_{}", missing_block.height);
            output.push(porcelain_field(
                &format!("{prefix}.height"),
                missing_block.height,
            ));
            output.push(porcelain_field(
                &format!("{prefix}.reason"),
                &missing_block.reason,
            ));
            if let Some(ref block_id) = missing_block.block_id {
                output.push(porcelain_field(&format!("{prefix}.block_id"), block_id));
            }
        }

        output.join("\n")
    }
}

impl Formattable for MissingBlockInfo {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();
        output.push(porcelain_field("height", self.height));
        output.push(porcelain_field("reason", &self.reason));
        if let Some(ref block_id) = self.block_id {
            output.push(porcelain_field("block_id", block_id));
        }
        output.join("\n")
    }
}
