use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{DatabaseBackend, L1Database};
use strata_identifiers::L1Height;
use strata_ol_chain_types::AsmManifest;
use strata_primitives::l1::L1BlockId;

use crate::{
    cli::OutputFormat,
    output::{
        l1::{L1BlockInfo, L1SummaryInfo, MissingBlockInfo},
        output,
    },
    utils::block_id::parse_l1_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-block")]
/// Get L1 block data from ASM manifest storage
pub(crate) struct GetL1BlockArgs {
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
    pub(crate) height_from: L1Height,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get the L1 chain tip (height, block_id) of the canonical chain tip.
pub(crate) fn get_l1_chain_tip(
    db: &impl DatabaseBackend,
) -> Result<(L1Height, L1BlockId), DisplayedError> {
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
    height: L1Height,
    mk_error: impl FnOnce(L1Height) -> DisplayedError,
) -> Result<L1BlockId, DisplayedError> {
    db.l1_db()
        .get_canonical_blockid_at_height(height)
        .internal_error(format!("Failed to get L1 block ID at height {height}"))?
        .ok_or_else(|| mk_error(height))
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

/// Get L1 block by block ID.
pub(crate) fn get_l1_block(
    db: &impl DatabaseBackend,
    args: GetL1BlockArgs,
) -> Result<(), DisplayedError> {
    let block_id = parse_l1_block_id(&args.block_id)?;
    let Some(l1_block_manifest) = get_l1_block_manifest(db, block_id)? else {
        return Ok(());
    };
    let prev_block_id = if l1_block_manifest.height() == 0 {
        None
    } else {
        Some(get_l1_block_id_at_height(
            db,
            l1_block_manifest.height().saturating_sub(1),
            |h| {
                DisplayedError::InternalError(
                    "No canonical L1 block found at height".to_string(),
                    Box::new(h),
                )
            },
        )?)
    };
    let block_info = L1BlockInfo::from_manifest(&block_id, &l1_block_manifest, prev_block_id);

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
    if start_height > l1_tip_height {
        return Err(DisplayedError::UserError(
            format!("Provided height is above canonical L1 tip {l1_tip_height}"),
            Box::new(start_height),
        ));
    }

    let start_block_id = get_l1_block_id_at_height(db, start_height, |h| {
        DisplayedError::UserError(
            "Provided height is not present in canonical L1 chain".to_string(),
            Box::new(h),
        )
    })?;

    let mut missing_blocks = Vec::new();
    let mut has_missing_manifest = false;
    for l1_height in start_height..=l1_tip_height {
        let maybe_block_id = l1_db
            .get_canonical_blockid_at_height(l1_height)
            .internal_error(format!(
                "Failed to get canonical block ID at height {l1_height}"
            ))?;

        let Some(block_id) = maybe_block_id else {
            missing_blocks.push(MissingBlockInfo {
                height: l1_height,
                reason: "Missing canonical block".to_string(),
                block_id: None,
            });
            continue;
        };

        has_missing_manifest |= l1_db
            .get_block_manifest(block_id)
            .internal_error(format!(
                "Failed to get L1 block manifest at height {l1_height}"
            ))?
            .is_none();
    }

    let output_data = L1SummaryInfo {
        tip_height: l1_tip_height,
        tip_block_id: l1_tip_block_id,
        from_height: start_height,
        from_block_id: start_block_id,
        expected_block_count: u64::from(l1_tip_height.saturating_sub(start_height) + 1),
        all_manifests_present: !has_missing_manifest,
        missing_blocks,
    };

    output(&output_data, args.output_format)
}
