use argh::FromArgs;
use hex::FromHex;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, Database, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId};
use strata_state::{block::L1Segment, header::L2BlockHeader};

use crate::cli::OutputFormat;

/// Shows details about a specific L2 block
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-l2-block")]
pub(crate) struct GetL2BlockArgs {
    /// L2 Block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// L2 Block information displayed to the user
#[derive(serde::Serialize)]
struct L2BlockInfo<'a> {
    id: &'a L2BlockId,
    status: &'a BlockStatus,
    header: &'a L2BlockHeader,
    l1_segment: &'a L1Segment,
}

/// Show details about a specific L2 block.
pub(crate) fn get_l2_block(db: &impl Database, args: GetL2BlockArgs) -> Result<(), DisplayedError> {
    // Convert String to L2BlockId
    let hex_str = args.block_id.strip_prefix("0x").unwrap_or(&args.block_id);
    if hex_str.len() != 64 {
        return Err(DisplayedError::UserError(
            "Block-id must be 32-byte / 64-char hex".into(),
            Box::new(args.block_id.to_owned()),
        ));
    }

    let bytes: [u8; 32] =
        <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))?;
    let block_id = L2BlockId::from(Buf32::from(bytes));

    // Fetch block status and data
    let status = db
        .l2_db()
        .get_block_status(block_id)
        .internal_error("Failed to read block status")?
        .unwrap_or(BlockStatus::Unchecked);

    let bundle = db
        .l2_db()
        .get_block_data(block_id)
        .internal_error("Failed to read block data")?
        .ok_or_else(|| {
            DisplayedError::UserError("block with id not found".to_string(), Box::new(block_id))
        })?;

    // Print status and header
    let l2_block = bundle.block();
    if args.output_format == OutputFormat::Json {
        let block_info = L2BlockInfo {
            id: &block_id,
            status: &status,
            header: l2_block.header().header(),
            l1_segment: l2_block.l1_segment(),
        };
        println!("{}", serde_json::to_string_pretty(&block_info).unwrap());
    } else {
        println!("L2 block id: {block_id:?}, status: {status:?}");
        println!("Block header: {:#?}", l2_block.header().header());
        println!("L1 segment");
        for l1_manifest in l2_block.body().l1_segment().new_manifests().iter() {
            println!(
                "L1 blkid {:?}, height {}, txs {}",
                l1_manifest.blkid(),
                l1_manifest.height(),
                l1_manifest.txs().len()
            );
        }
    }

    Ok(())
}
