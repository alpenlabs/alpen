//! L1 block formatting implementations

use strata_ol_chain_types::AsmManifest;
use strata_primitives::l1::L1BlockId;

use super::{helpers::porcelain_field, traits::Formattable};

/// L1 block information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct L1BlockInfo<'a> {
    pub(crate) block_id: &'a L1BlockId,
    pub(crate) height: u32,
    pub(crate) logs_count: usize,
}

impl<'a> L1BlockInfo<'a> {
    /// Create L1BlockInfo from an AsmManifest
    pub(crate) fn from_manifest(block_id: &'a L1BlockId, manifest: &AsmManifest) -> Self {
        Self {
            block_id,
            height: manifest.height(),
            logs_count: manifest.logs().len(),
        }
    }
}

/// L1 summary information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct L1SummaryInfo {
    pub(crate) tip_height: u32,
    pub(crate) tip_block_id: String,
    pub(crate) from_height: u64,
    pub(crate) from_block_id: String,
    pub(crate) expected_block_count: u64,
    pub(crate) all_manifests_present: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) missing_blocks: Vec<MissingBlockInfo>,
}

/// Information about missing blocks
#[derive(serde::Serialize)]
pub(crate) struct MissingBlockInfo {
    pub(crate) height: u32,
    pub(crate) reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) block_id: Option<String>,
}

impl<'a> Formattable for L1BlockInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field("block_id", format!("{:?}", self.block_id)));
        output.push(porcelain_field("height", self.height));
        output.push(porcelain_field("logs_count", self.logs_count));

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
