use bitcoin::{consensus::serialize, hashes::Hash, Block};
use bitcoind_async_client::traits::Reader;
use strata_btc_types::legacy::{generate_l1_tx, L1BlockManifest, L1HeaderRecord, L1Tx};
use strata_identifiers::Buf32;
use strata_primitives::l1::L1BlockCommitment;
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
    epoch: u64,
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

    let txs: Vec<_> = generate_l1txs(&blockdata);
    let num_txs = txs.len();
    let manifest = generate_block_manifest(blockdata.block(), txs, epoch, height);
    let l1blockid = *manifest.blkid();

    storage.l1().put_block_data_async(manifest).await?;
    storage
        .l1()
        .extend_canonical_chain_async(&l1blockid)
        .await?;
    info!(%height, %l1blockid, txs = %num_txs, "wrote L1 block manifest");

    // Create a sync event if it's something we care about.
    let blkid: Buf32 = blockdata.block().block_hash().into();
    Ok(Option::Some(
        L1BlockCommitment::from_height_u64(height, blkid.into()).expect("valid height"),
    ))
}

/// Given a block, generates a manifest of the parts we care about that we can
/// store in the database.
fn generate_block_manifest(
    block: &Block,
    txs: Vec<L1Tx>,
    epoch: u64,
    height: u64,
) -> L1BlockManifest {
    let blockid = block.block_hash().into();
    let root = block
        .witness_root()
        .map(|x| x.to_byte_array())
        .unwrap_or_default();
    let header = serialize(&block.header);

    let rec = L1HeaderRecord::new(blockid, header, Buf32::from(root));
    L1BlockManifest::new(rec, txs, epoch, height)
}

fn generate_l1txs(blockdata: &BlockData) -> Vec<L1Tx> {
    blockdata
        .relevant_txs()
        .iter()
        .map(|tx_entry| {
            generate_l1_tx(
                blockdata.block(),
                *tx_entry.index(),
                tx_entry.item().protocol_ops().to_vec(),
            )
        })
        .collect()
}
