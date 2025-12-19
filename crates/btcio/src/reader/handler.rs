use bitcoin::hashes::Hash;
use bitcoind_async_client::traits::Reader;
use strata_asm_common::AsmManifest;
use strata_asm_spec::StrataAsmSpec;
use strata_asm_stf::{pre_process_asm, AsmStfInput};
use strata_identifiers::{Buf32, Epoch, L1BlockCommitment};
use strata_state::BlockSubmitter;
use tracing::*;

use super::{
    event::{BlockData, L1Event},
    query::ReaderContext,
};

pub(crate) async fn handle_bitcoin_event<R: Reader>(
    event: L1Event,
    ctx: &ReaderContext<R>,
    block_submitter: &impl BlockSubmitter,
) -> anyhow::Result<()> {
    let new_block = match event {
        L1Event::RevertTo(block) => {
            // L1 reorgs will be handled in L2 STF, we just have to reflect
            // what the client is telling us in the database.
            let height = block.height_u64();
            ctx.storage
                .l1()
                .revert_canonical_chain_async(height)
                .await?;
            debug!(%height, "reverted L1 block database");
            // We don't submit events related to reverts,
            // as long as we updated canonical chain in the db.
            Option::None
        }

        L1Event::BlockData(blockdata, epoch) => handle_blockdata(ctx, blockdata, epoch).await?,
    };

    // Dispatch new blocks.
    if let Some(block) = new_block {
        block_submitter.submit_block_async(block).await?;
    }
    Ok(())
}

async fn handle_blockdata<R: Reader>(
    ctx: &ReaderContext<R>,
    blockdata: BlockData,
    _epoch: Epoch,
) -> anyhow::Result<Option<L1BlockCommitment>> {
    let ReaderContext {
        params, storage, ..
    } = ctx;

    let height = blockdata.block_num();

    // Bail out fast if we don't have to care.
    let genesis = params.rollup().genesis_l1_view.height_u64();
    if height < genesis {
        warn!(%height, %genesis, "ignoring BlockData for block before genesis");
        return Ok(Option::None);
    }

    let block = blockdata.block();
    let blkid: Buf32 = block.block_hash().into();
    let l1blockid = blkid.into();

    // Get current ASM state from storage
    let asm_state = storage
        .asm()
        .fetch_most_recent_state()?
        .map(|(_, state)| state)
        .ok_or_else(|| anyhow::anyhow!("No ASM state available"))?;

    // Create ASM spec
    let asm_spec = StrataAsmSpec::from_params(params.rollup());

    // Pre-process the block through ASM STF
    let pre_process = pre_process_asm(&asm_spec, asm_state.state(), block)?;

    // For now, we create a minimal aux_data - full resolution happens in ASM worker
    let aux_data = Default::default();

    // Get witness root
    let wtxids_root: Buf32 = block
        .witness_root()
        .map(|root| root.as_raw_hash().to_byte_array())
        .unwrap_or_else(|| block.header.merkle_root.as_raw_hash().to_byte_array())
        .into();

    let stf_input = AsmStfInput {
        protocol_txs: pre_process.txs,
        header: &block.header,
        wtxids_root,
        aux_data,
    };

    // Compute ASM transition to get manifest
    let stf_output =
        strata_asm_stf::compute_asm_transition(&asm_spec, asm_state.state(), stf_input)?;
    let manifest: AsmManifest = stf_output.manifest;
    let num_logs = manifest.logs().len();

    // Store manifest in L1 database
    storage.l1().put_block_data_async(manifest, height).await?;
    storage
        .l1()
        .extend_canonical_chain_async(&l1blockid, height)
        .await?;
    info!(%height, %l1blockid, logs = %num_logs, "wrote L1 ASM manifest");

    // Create a sync event
    Ok(Option::Some(
        L1BlockCommitment::from_height_u64(height, blkid.into()).expect("valid height"),
    ))
}
