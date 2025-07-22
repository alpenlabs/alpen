use bitcoin::{consensus::serialize, hashes::Hash, Block};
use bitcoind_async_client::traits::Reader;
use strata_primitives::{
    buf::Buf32,
    l1::{
        generate_l1_tx, HeaderVerificationState, L1BlockCommitment, L1BlockManifest,
        L1HeaderRecord, L1Tx,
    },
};
use strata_state::sync_event::{EventSubmitter, SyncEvent};
use tracing::*;

use super::{
    event::{BlockData, L1Event},
    query::ReaderContext,
};

pub(crate) async fn handle_bitcoin_event<R: Reader>(
    event: L1Event,
    ctx: &ReaderContext<R>,
    event_submitter: &impl EventSubmitter,
) -> anyhow::Result<()> {
    let sync_evs = match event {
        L1Event::RevertTo(block) => {
            // L1 reorgs will be handled in L2 STF, we just have to reflect
            // what the client is telling us in the database.
            let height = block.height();
            let sync_event = SyncEvent::L1Revert(block);
            let sync_event_idx = ctx
                .storage
                .l1()
                .revert_canonical_chain_and_write_sync_event_async(height, sync_event)
                .await?;
            debug!(%height, "reverted L1 block database");
            vec![sync_event_idx]
        }

        L1Event::BlockData(blockdata, epoch, hvs) => {
            handle_blockdata(ctx, blockdata, hvs, epoch).await?
        }
    };

    // Write to sync event db.
    for ev in sync_evs {
        event_submitter.submit_event_idx_async(ev).await?;
    }
    Ok(())
}

async fn handle_blockdata<R: Reader>(
    ctx: &ReaderContext<R>,
    blockdata: BlockData,
    hvs: Option<HeaderVerificationState>,
    epoch: u64,
) -> anyhow::Result<Vec<SyncEvent>> {
    let ReaderContext {
        params, storage, ..
    } = ctx;

    let height = blockdata.block_num();
    let mut sync_evs = Vec::new();

    // Bail out fast if we don't have to care.
    let horizon = params.rollup().horizon_l1_height;
    if height < horizon {
        warn!(%height, %horizon, "ignoring BlockData for block before horizon");
        return Ok(sync_evs);
    }

    let txs: Vec<_> = generate_l1txs(&blockdata);
    let num_txs = txs.len();
    let manifest = generate_block_manifest(blockdata.block(), hvs, txs, epoch, height);
    let l1blockid = *manifest.blkid();

    storage.l1().put_block_data_async(manifest).await?;
    storage
        .l1()
        .extend_canonical_chain_async(&l1blockid)
        .await?;
    info!(%height, %l1blockid, txs = %num_txs, "wrote L1 block manifest");

    // Create a sync event if it's something we care about.
    let blkid: Buf32 = blockdata.block().block_hash().into();
    let block_commitment = L1BlockCommitment::new(height, blkid.into());
    let sync_event = SyncEvent::L1Block(block_commitment);
    let sync_event_idx = storage
        .l1()
        .extend_canonical_chain_and_write_sync_event_async(&l1blockid, sync_event)
        .await?;
    info!(%height, %l1blockid, txs = %num_txs, sync_ev = %sync_event_idx, "wrote L1 block manifest");

    sync_evs.push(sync_event_idx);

    Ok(sync_evs)
}

/// Given a block, generates a manifest of the parts we care about that we can
/// store in the database.
fn generate_block_manifest(
    block: &Block,
    hvs: Option<HeaderVerificationState>,
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
    L1BlockManifest::new(rec, hvs, txs, epoch, height)
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
