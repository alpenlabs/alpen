use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{DatabaseBackend, L1Database};
use strata_ol_chain_types::AsmManifest;
use strata_primitives::l1::L1BlockId;

use crate::{
    cli::OutputFormat,
    output::{
        l1::{L1BlockInfo, L1SummaryInfo},
        output,
    },
    utils::block_id::parse_l1_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-manifest")]
/// Get L1 manifest
pub(crate) struct GetL1ManifestArgs {
    /// block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-summary")]
/// Get L1 summary
pub(crate) struct GetL1SummaryArgs {
    /// l1 height to look up the summary about
    #[argh(positional)]
    pub(crate) height_from: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get the L1 chain tip (height, block_id) of the canonical chain tip.
pub(crate) fn get_l1_chain_tip(
    db: &impl DatabaseBackend,
) -> Result<(u64, L1BlockId), DisplayedError> {
    db.l1_db()
        .get_canonical_chain_tip()
        .internal_error("Failed to get L1 tip")?
        .ok_or_else(|| {
            DisplayedError::InternalError("L1 tip not found in database".to_string(), Box::new(()))
        })
}

/// Get L1 block ID at a specific height.
pub(crate) fn get_l1_block_id_at_height(
    db: &impl DatabaseBackend,
    height: u64,
) -> Result<L1BlockId, DisplayedError> {
    db.l1_db()
        .get_canonical_blockid_at_height(height)
        .internal_error(format!("Failed to get L1 block ID at height {height}"))?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "L1 block id not found for height".to_string(),
                Box::new(height),
            )
        })
}

/// Get L1 block manifest by block ID.
pub(crate) fn get_l1_block_manifest(
    db: &impl DatabaseBackend,
    block_id: L1BlockId,
) -> Result<Option<AsmManifest>, DisplayedError> {
    db.l1_db()
        .get_block_manifest(block_id)
        .internal_error(format!("Failed to get block manifest for id {block_id:?}",))
}

/// Get L1 manifest by block ID.
pub(crate) fn get_l1_manifest(
    db: &impl DatabaseBackend,
    args: GetL1ManifestArgs,
) -> Result<(), DisplayedError> {
    // Parse block ID using utility function
    let block_id = parse_l1_block_id(&args.block_id)?;

    // Get block manifest using helper function
    let l1_block_manifest = get_l1_block_manifest(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No L1 block manifest found for block id".to_string(),
            Box::new(block_id),
        )
    })?;

    // Create the output data structure using the new ASM manifest
    let block_info = L1BlockInfo::from_manifest(&block_id, &l1_block_manifest);

    // Use the output utility
    output(&block_info, args.output_format)
}

/// Get L1 summary - check all L1 block manifests exist in database.
pub(crate) fn get_l1_summary(
    db: &impl DatabaseBackend,
    args: GetL1SummaryArgs,
) -> Result<(), DisplayedError> {
    let l1_db = db.l1_db();

    // Use helper function to get L1 tip
    let (l1_tip_height, l1_tip_block_id) = get_l1_chain_tip(db)?;

    let start_height = args.height_from;
    let start_block_id = get_l1_block_id_at_height(db, start_height)?;

    // Check if all L1 blocks from L1 horizon to tip are present
    let mut missing_heights = Vec::new();
    let all_l1_manifests_present = (start_height..=l1_tip_height).all(|l1_height| {
        let Some(block_id) = l1_db
            .get_canonical_blockid_at_height(l1_height)
            .ok()
            .flatten()
        else {
            missing_heights.push(l1_height);
            return false;
        };

        if l1_db.get_block_manifest(block_id).ok().flatten().is_none() {
            missing_heights.push(l1_height);
            return false;
        }

        true
    });

    let output_data = L1SummaryInfo {
        tip_height: l1_tip_height,
        tip_block_id: format!("{l1_tip_block_id:?}"),
        from_height: start_height,
        from_block_id: format!("{start_block_id:?}"),
        expected_block_count: l1_tip_height.saturating_sub(start_height) + 1,
        all_manifests_present: all_l1_manifests_present,
        missing_blocks: missing_heights
            .into_iter()
            .map(|height| crate::output::l1::MissingBlockInfo {
                height,
                reason: "Missing block".to_string(),
                block_id: None,
            })
            .collect(),
    };

    output(&output_data, args.output_format)
}
