use bitcoind_async_client::traits::Reader;
use strata_btc_types::BlockHashExt;
use strata_identifiers::{Epoch, L1BlockCommitment};
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
        btcio_params,
        storage,
        ..
    } = ctx;

    let height = blockdata.block_num();

    // Bail out fast if we don't have to care.
    let genesis = btcio_params.genesis_l1_height();
    if height < genesis {
        warn!(%height, %genesis, "ignoring BlockData for block before genesis");
        return Ok(Option::None);
    }

    let block = blockdata.block();
    let l1blockid = block.block_hash().to_l1_block_id();

    // Store chain tracking data only - ASM worker will handle manifest creation
    storage
        .l1()
        .extend_canonical_chain_async(&l1blockid, height)
        .await?;
    info!(%height, %l1blockid, "stored L1 chain tracking data");

    // Create a sync event - the ASM worker will listen to this and create manifests
    Ok(Option::Some(
        L1BlockCommitment::from_height_u64(height, l1blockid).expect("valid height"),
    ))
}
