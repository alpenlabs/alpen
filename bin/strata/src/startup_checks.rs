use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use bitcoin::{BlockHash, Network};
use bitcoind_async_client::{
    Client, corepc_types::model::GetBlockchainInfo, error::ClientError, traits::Reader,
};
use strata_btc_types::BlockHashExt;
use strata_btcio::{is_bitcoind_warmup_error, is_block_height_out_of_range_error};
use strata_checkpoint_types::EpochSummary;
use strata_db_types::ol_block::BlockStatus;
use strata_identifiers::{EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_node_context::NodeContext;
use strata_ol_chain_types::{OLBlock, OLBlockHeader};
use strata_primitives::L1BlockCommitment;
use strata_storage::NodeStorage;
use tracing::{info, warn};

#[async_trait]
pub(crate) trait StartupBitcoinClient {
    async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo>;
    async fn get_block_hash_for_startup(&self, height: u64) -> Result<BlockHash>;
}

#[async_trait]
impl StartupBitcoinClient for Client {
    async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo> {
        Reader::get_blockchain_info(self).await.map_err(Into::into)
    }

    async fn get_block_hash_for_startup(&self, height: u64) -> Result<BlockHash> {
        Reader::get_block_hash(self, height)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartupBitcoinCheck {
    Verified,
    Deferred { reason: String },
}

fn is_retryable_startup_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<ClientError>().is_some_and(|err| {
            err.is_retriable()
                || is_bitcoind_warmup_error(err)
                || is_block_height_out_of_range_error(err)
        })
    })
}

pub(crate) async fn run_bitcoin_connectivity_and_network_checks(
    bitcoin_client: &impl StartupBitcoinClient,
    expected_network: Network,
) -> Result<StartupBitcoinCheck> {
    let chain_info = match bitcoin_client.get_blockchain_info_for_startup().await {
        Ok(chain_info) => chain_info,
        Err(err) if is_retryable_startup_error(&err) => {
            return Ok(StartupBitcoinCheck::Deferred {
                reason: format!(
                    "startup: could not connect to bitcoind via getblockchaininfo: {err}"
                ),
            });
        }
        Err(err) => {
            return Err(err)
                .context("startup: could not connect to bitcoind via getblockchaininfo");
        }
    };

    if chain_info.chain != expected_network {
        bail!(
            "startup: bitcoind network mismatch: expected {}, got {}",
            expected_network,
            chain_info.chain
        );
    }

    Ok(StartupBitcoinCheck::Verified)
}

pub(crate) async fn verify_l1_anchor_block(
    bitcoin_client: &impl StartupBitcoinClient,
    expected_l1_block: L1BlockCommitment,
) -> Result<StartupBitcoinCheck> {
    let actual_hash = match bitcoin_client
        .get_block_hash_for_startup(expected_l1_block.height() as u64)
        .await
    {
        Ok(actual_hash) => actual_hash,
        Err(err) if is_retryable_startup_error(&err) => {
            return Ok(StartupBitcoinCheck::Deferred {
                reason: format!(
                    "startup: failed to fetch L1 block hash from bitcoind at height {}: {err}",
                    expected_l1_block.height()
                ),
            });
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "startup: failed to fetch L1 block hash from bitcoind at height {}",
                    expected_l1_block.height()
                )
            });
        }
    };

    let actual_block_id = actual_hash.to_l1_block_id();
    if actual_block_id != *expected_l1_block.blkid() {
        bail!(
            "startup: genesis L1 block hash mismatch at height {height}: expected {expected}, got {actual_block_id}",
            height = expected_l1_block.height(),
            expected = expected_l1_block.blkid(),
        );
    }

    Ok(StartupBitcoinCheck::Verified)
}

fn get_ol_genesis_block(storage: &NodeStorage) -> Result<Option<OLBlockCommitment>> {
    let genesis_commitment = storage
        .ol_block()
        .get_blocks_at_height_blocking(0)
        .context("startup: failed to query OL blocks at genesis slot 0")?
        .first()
        .copied()
        .map(|blkid| OLBlockCommitment::new(0, blkid));

    Ok(genesis_commitment)
}

/// Ensures the genesis canonical entry exists and matches `genesis_commitment`.
///
/// Runs on every node before the canonical tip checks. Genesis init seeds this
/// entry, but that happens later in service startup, so a legacy or
/// checkpoint-sync DB holding OL genesis block data may still lack the canonical
/// index here; this repairs it so the canonical tip resolves to genesis.
fn ensure_genesis_canonical_entry(
    storage: &NodeStorage,
    genesis_commitment: OLBlockCommitment,
) -> Result<()> {
    let canonical_genesis = storage
        .ol_block()
        .get_canonical_block_at_blocking(0)
        .context("startup: failed to query canonical OL genesis block")?;
    if let Some(canonical_genesis) = canonical_genesis {
        if canonical_genesis != genesis_commitment {
            bail!(
                "startup: canonical OL genesis block mismatch: expected {genesis_commitment}, got {canonical_genesis}"
            );
        }
        return Ok(());
    }

    storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(0, vec![*genesis_commitment.blkid()])
        .context("startup: failed to seed genesis canonical OL block entry")
}

/// Ensures the canonical block index is initialized for legacy DBs.
///
/// This backfills DBs created before the canonical OL block index existed. FCM
/// reconciles the unfinalized suffix after startup once the finalized base is
/// present in the canonical index.
fn ensure_canonical_block_index(
    storage: &NodeStorage,
    genesis_commitment: OLBlockCommitment,
    finalized_epoch: Option<EpochCommitment>,
) -> Result<()> {
    let finalized_commitment = finalized_epoch
        .filter(|epoch| !epoch.is_null())
        .map_or(genesis_commitment, |epoch| epoch.to_block_commitment());
    let canonical_finalized = storage
        .ol_block()
        .get_canonical_block_at_blocking(finalized_commitment.slot())
        .with_context(|| {
            format!("startup: failed to query canonical finalized OL block {finalized_commitment}")
        })?;
    if canonical_finalized == Some(finalized_commitment)
        && let Ok(Some(canonical_tip)) = storage.ol_block().get_canonical_tip_blocking()
        && canonical_tip.slot() >= finalized_commitment.slot()
    {
        return Ok(());
    }

    // Keep this as a single suffix replacement so restart never observes a partially migrated
    // finalized prefix. For scale: 10 million slots is about 305 MiB for Vec<OLBlockId> (32 bytes
    // each) plus about 381 MiB for the temporary Vec<(Slot, OLBlockId)> built by the DB layer. A
    // legacy DB has an empty canonical tree, so the suffix-removal side is effectively free. If
    // this ever needs batching, the migration must stay resumable and must not let FCM start until
    // canonical_at(finalized_slot) equals the declared finalized block.
    let finalized_chain =
        collect_finalized_canonical_chain(storage, genesis_commitment, finalized_commitment)
            .context("startup: failed to collect finalized OL canonical blocks")?;

    storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(0, finalized_chain)
        .context("startup: failed to backfill canonical OL block index")?;

    Ok(())
}

fn collect_finalized_canonical_chain(
    storage: &NodeStorage,
    genesis_commitment: OLBlockCommitment,
    finalized_commitment: OLBlockCommitment,
) -> Result<Vec<OLBlockId>> {
    let mut reversed_chain = Vec::new();
    let mut current = finalized_commitment;

    loop {
        let blkid = *current.blkid();
        let block = storage
            .ol_block()
            .get_block_data_blocking(blkid)
            .with_context(|| format!("startup: failed to query finalized OL block {current}"))?
            .ok_or_else(|| anyhow!("startup: missing finalized OL block {current}"))?;

        if block.header().slot() != current.slot() {
            bail!(
                "startup: finalized OL block slot mismatch: commitment {current}, block slot {}",
                block.header().slot()
            );
        }

        let status = storage
            .ol_block()
            .get_block_status_blocking(blkid)
            .with_context(|| {
                format!("startup: failed to query finalized OL block status {current}")
            })?;
        if status != Some(BlockStatus::Valid) {
            bail!("startup: finalized OL block {current} is not valid");
        }

        reversed_chain.push(blkid);

        if current.slot() == 0 {
            if current != genesis_commitment {
                bail!(
                    "startup: finalized OL chain terminates at non-genesis block: expected {genesis_commitment}, got {current}"
                );
            }
            if !block.header().parent_blkid().is_null() {
                bail!("startup: genesis OL block must have null parent commitment");
            }
            break;
        }

        let parent_slot = current.slot() - 1;
        current = OLBlockCommitment::new(parent_slot, *block.header().parent_blkid());
    }

    reversed_chain.reverse();
    Ok(reversed_chain)
}

fn collect_finalized_canonical_chain_from_history_base(
    storage: &NodeStorage,
    history_base: EpochCommitment,
    finalized_commitment: OLBlockCommitment,
) -> Result<Vec<OLBlockId>> {
    let base_commitment = history_base.to_block_commitment();
    let mut reversed_chain = Vec::new();
    let mut current = finalized_commitment;

    loop {
        if current.slot() < base_commitment.slot() {
            bail!(
                "startup: finalized OL chain terminated below history base: expected to reach {base_commitment}, got {current}"
            );
        }

        if current.slot() == base_commitment.slot() {
            if current != base_commitment {
                bail!(
                    "startup: finalized OL history-base linkage mismatch: expected {base_commitment}, got {current}"
                );
            }

            let header = storage
                .ol_block()
                .get_terminal_header_blocking(*current.blkid())
                .with_context(|| {
                    format!(
                        "startup: failed to query finalized OL history-base terminal header {current}"
                    )
                })?
                .ok_or_else(|| {
                    anyhow!("startup: missing finalized OL history-base terminal header {current}")
                })?;

            if header.slot() != current.slot() {
                bail!(
                    "startup: finalized OL history-base header slot mismatch: expected {}, got {}",
                    current.slot(),
                    header.slot()
                );
            }

            let header_blkid = header.compute_blkid();
            if header_blkid != *current.blkid() {
                bail!(
                    "startup: finalized OL history-base header block ID mismatch: expected {}, got {header_blkid}",
                    current.blkid()
                );
            }

            reversed_chain.push(*current.blkid());
            break;
        }

        let blkid = *current.blkid();
        let block = storage
            .ol_block()
            .get_block_data_blocking(blkid)
            .with_context(|| format!("startup: failed to query finalized OL block {current}"))?
            .ok_or_else(|| anyhow!("startup: missing finalized OL block {current}"))?;

        if block.header().slot() != current.slot() {
            bail!(
                "startup: finalized OL block slot mismatch: commitment {current}, block slot {}",
                block.header().slot()
            );
        }

        let status = storage
            .ol_block()
            .get_block_status_blocking(blkid)
            .with_context(|| {
                format!("startup: failed to query finalized OL block status {current}")
            })?;
        if status != Some(BlockStatus::Valid) {
            bail!("startup: finalized OL block {current} is not valid");
        }

        reversed_chain.push(blkid);
        current = OLBlockCommitment::new(current.slot() - 1, *block.header().parent_blkid());
    }

    reversed_chain.reverse();
    Ok(reversed_chain)
}

fn ensure_canonical_block_index_from_history_base(
    storage: &NodeStorage,
    history_base: EpochCommitment,
    finalized_epoch: Option<EpochCommitment>,
) -> Result<()> {
    let base_commitment = history_base.to_block_commitment();
    let finalized_commitment = finalized_epoch
        .filter(|epoch| !epoch.is_null())
        .map_or(base_commitment, |epoch| epoch.to_block_commitment());
    let canonical_finalized = storage
        .ol_block()
        .get_canonical_block_at_blocking(finalized_commitment.slot())
        .with_context(|| {
            format!("startup: failed to query canonical finalized OL block {finalized_commitment}")
        })?;
    if canonical_finalized == Some(finalized_commitment)
        && let Ok(Some(canonical_tip)) = storage.ol_block().get_canonical_tip_blocking()
        && canonical_tip.slot() >= finalized_commitment.slot()
    {
        return Ok(());
    }

    let finalized_chain = collect_finalized_canonical_chain_from_history_base(
        storage,
        history_base,
        finalized_commitment,
    )
    .context("startup: failed to collect finalized OL canonical blocks")?;

    storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(base_commitment.slot(), finalized_chain)
        .context("startup: failed to backfill canonical OL block index")
}

/// Verifies that OL state exists for the resolved genesis block commitment.
fn verify_genesis_ol_state(
    storage: &NodeStorage,
    genesis_commitment: OLBlockCommitment,
) -> Result<()> {
    let has_genesis_state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(genesis_commitment)
        .context("startup: failed to query OL state for genesis block")?
        .is_some();

    if !has_genesis_state {
        bail!("startup: missing genesis OL state for slot 0 block");
    }

    Ok(())
}

/// Verifies that a canonical genesis epoch summary exists at epoch 0.
fn verify_genesis_epoch_summary(
    storage: &NodeStorage,
    genesis_commitment: OLBlockCommitment,
) -> Result<()> {
    let genesis_epoch_commitment = EpochCommitment::new(0, 0, *genesis_commitment.blkid());
    let has_genesis_summary = storage
        .ol_checkpoint()
        .get_epoch_summary_blocking(genesis_epoch_commitment)
        .context("startup: failed to query genesis epoch summary (epoch 0)")?
        .is_some();

    if !has_genesis_summary {
        bail!("startup: missing genesis epoch summary for epoch 0");
    }

    Ok(())
}

fn validate_persisted_state_presence(has_client_state: bool, has_ol_genesis: bool) -> Result<()> {
    match (has_client_state, has_ol_genesis) {
        (true, false) => bail!(
            "startup: inconsistent persisted state: client state exists but OL genesis block is missing"
        ),
        (false, true) => bail!(
            "startup: inconsistent persisted state: OL genesis block exists but client state is missing"
        ),
        _ => Ok(()),
    }
}

/// Verifies that a canonical OL tip block exists and returns its commitment.
fn resolve_tip_ol_block(storage: &NodeStorage) -> Result<OLBlockCommitment> {
    storage
        .ol_block()
        .get_canonical_tip_blocking()
        .context("startup: failed to resolve canonical OL tip block")?
        .ok_or_else(|| anyhow!("startup: missing canonical OL tip block"))
}

/// Verifies that OL block data exists for the resolved tip commitment.
fn verify_tip_ol_block(
    storage: &NodeStorage,
    tip_commitment: OLBlockCommitment,
) -> Result<OLBlock> {
    storage
        .ol_block()
        .get_block_data_blocking(*tip_commitment.blkid())
        .context("startup: failed to query OL tip block data")?
        .ok_or_else(|| anyhow!("startup: missing OL tip block data"))
}

/// Verifies that the tip's parent block exists unless the tip is genesis.
fn verify_tip_parent(
    storage: &NodeStorage,
    tip_block: &OLBlock,
    tip_commitment: OLBlockCommitment,
) -> Result<()> {
    if tip_commitment.slot() == 0 {
        if !tip_block.header().parent_blkid().is_null() {
            bail!("startup: genesis tip block must have null parent commitment");
        }
        return Ok(());
    }

    let parent_blkid = *tip_block.header().parent_blkid();
    let has_parent = storage
        .ol_block()
        .get_block_data_blocking(parent_blkid)
        .context("startup: failed to query OL tip parent block data")?
        .is_some();

    if !has_parent {
        bail!("startup: missing OL parent block for non-genesis tip");
    }

    Ok(())
}

/// Verifies that the tip's parent header exists unless the tip is genesis.
fn verify_tip_parent_header(
    storage: &NodeStorage,
    tip_block: &OLBlock,
    tip_commitment: OLBlockCommitment,
) -> Result<()> {
    if tip_commitment.slot() == 0 {
        if !tip_block.header().parent_blkid().is_null() {
            bail!("startup: genesis tip block must have null parent commitment");
        }
        return Ok(());
    }

    let parent_blkid = *tip_block.header().parent_blkid();
    let has_parent = storage
        .ol_block()
        .get_ol_header_blocking(parent_blkid)
        .context("startup: failed to query OL tip parent block data")?
        .is_some();

    if !has_parent {
        bail!("startup: missing OL parent block for non-genesis tip");
    }

    Ok(())
}

/// Verifies that OL state exists for the resolved tip block commitment.
fn verify_tip_ol_state(storage: &NodeStorage, tip_commitment: OLBlockCommitment) -> Result<()> {
    let has_tip_state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(tip_commitment)
        .context("startup: failed to query OL state for tip block")?
        .is_some();

    if !has_tip_state {
        bail!("startup: missing OL state for tip block {}", tip_commitment);
    }

    Ok(())
}

/// Verifies that epoch summary exists for tip epoch - 1 when tip is post-genesis.
fn verify_previous_epoch_summary_for_tip(storage: &NodeStorage, tip_block: &OLBlock) -> Result<()> {
    verify_previous_epoch_summary_for_tip_header(storage, tip_block.header())
}

fn verify_previous_epoch_summary_for_tip_header(
    storage: &NodeStorage,
    tip_header: &OLBlockHeader,
) -> Result<()> {
    let tip_epoch = tip_header.epoch();
    if tip_epoch == 0 {
        return Ok(());
    }

    let previous_epoch = tip_epoch - 1;
    let previous_epoch_commitment = storage
        .ol_checkpoint()
        .get_canonical_epoch_commitment_at_blocking(previous_epoch)
        .context("startup: failed to query epoch commitment for previous epoch")?
        .ok_or_else(|| anyhow!("startup: missing epoch summary for previous epoch"))?;

    let has_summary = storage
        .ol_checkpoint()
        .get_epoch_summary_blocking(previous_epoch_commitment)
        .context("startup: failed to query previous epoch summary")?
        .is_some();

    if !has_summary {
        bail!("startup: missing epoch summary for previous epoch");
    }

    Ok(())
}

pub(crate) fn verify_anchor_header_summary(
    history_base: EpochCommitment,
    header: &OLBlockHeader,
    summary: &EpochSummary,
) -> Result<()> {
    if header.slot() != summary.terminal().slot() {
        bail!(
            "startup: OL history anchor header slot mismatch: expected {}, got {}",
            summary.terminal().slot(),
            header.slot()
        );
    }

    if header.epoch() != summary.epoch() {
        bail!(
            "startup: OL history anchor header epoch mismatch: expected {}, got {}",
            summary.epoch(),
            header.epoch()
        );
    }

    let header_blkid = header.compute_blkid();
    if header_blkid != *summary.terminal().blkid() {
        bail!(
            "startup: OL history anchor header/summary block ID mismatch: expected {}, got {header_blkid}",
            summary.terminal().blkid()
        );
    }

    if header.state_root() != summary.final_state() {
        bail!(
            "startup: OL history anchor state root mismatch: expected {}, got {}",
            summary.final_state(),
            header.state_root()
        );
    }

    if summary.get_epoch_commitment() != history_base {
        bail!(
            "startup: OL history anchor summary commitment mismatch: expected {history_base}, got {}",
            summary.get_epoch_commitment()
        );
    }

    Ok(())
}

pub(crate) fn verify_anchor_summary_and_state(
    storage: &NodeStorage,
    history_base: EpochCommitment,
    header: &OLBlockHeader,
    summary: &EpochSummary,
) -> Result<()> {
    if !header.is_terminal() {
        bail!("startup: OL history anchor header is not terminal");
    }

    let canonical_epoch = storage
        .ol_checkpoint()
        .get_canonical_epoch_commitment_at_blocking(history_base.epoch())
        .context("startup: failed to query canonical OL history anchor epoch commitment")?
        .ok_or_else(|| anyhow!("startup: missing canonical OL history anchor epoch commitment"))?;
    if canonical_epoch != history_base {
        bail!(
            "startup: canonical OL history anchor epoch commitment mismatch: expected {history_base}, got {canonical_epoch}"
        );
    }

    verify_anchor_header_summary(history_base, header, summary)?;

    let anchor_commitment = history_base.to_block_commitment();
    let state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(anchor_commitment)
        .context("startup: failed to query OL history anchor state")?
        .ok_or_else(|| anyhow!("startup: missing OL history anchor state"))?;

    let state_slot = state.global_state().get_cur_slot();
    if state_slot != header.slot() {
        bail!(
            "startup: OL history anchor state slot mismatch: expected {}, got {state_slot}",
            header.slot()
        );
    }

    let expected_state_epoch = header.epoch().checked_add(1).ok_or_else(|| {
        anyhow!(
            "startup: OL history anchor header epoch {} cannot advance to the next epoch",
            header.epoch()
        )
    })?;
    let state_epoch = state.epoch_state().cur_epoch();
    if state_epoch != expected_state_epoch {
        bail!(
            "startup: OL history anchor state epoch mismatch: expected {expected_state_epoch}, got {state_epoch}"
        );
    }

    Ok(())
}

pub(crate) fn verify_history_anchor(
    storage: &NodeStorage,
    history_base: EpochCommitment,
) -> Result<OLBlockHeader> {
    let anchor_commitment = history_base.to_block_commitment();
    let header = storage
        .ol_block()
        .get_terminal_header_blocking(*anchor_commitment.blkid())
        .context("startup: failed to query OL history anchor terminal header")?
        .ok_or_else(|| anyhow!("startup: missing OL history anchor terminal header"))?;

    let summary = storage
        .ol_checkpoint()
        .get_epoch_summary_blocking(history_base)
        .context("startup: failed to query OL history anchor epoch summary")?
        .ok_or_else(|| anyhow!("startup: missing OL history anchor epoch summary"))?;
    verify_anchor_summary_and_state(storage, history_base, &header, &summary)?;

    Ok(header)
}

fn verify_tip_from_history_base(
    storage: &NodeStorage,
    tip_commitment: OLBlockCommitment,
    history_base: EpochCommitment,
) -> Result<()> {
    let anchor_commitment = history_base.to_block_commitment();
    if tip_commitment.slot() < anchor_commitment.slot() {
        bail!(
            "startup: canonical OL tip is below history base: expected slot at least {}, got {tip_commitment}",
            anchor_commitment.slot()
        );
    }
    if tip_commitment.slot() == anchor_commitment.slot() && tip_commitment != anchor_commitment {
        bail!(
            "startup: canonical OL tip at history-base slot does not match anchor: expected {anchor_commitment}, got {tip_commitment}"
        );
    }

    if tip_commitment == anchor_commitment {
        let anchor_header = verify_history_anchor(storage, history_base)?;
        return verify_previous_epoch_summary_for_tip_header(storage, &anchor_header);
    }

    let tip_block = verify_tip_ol_block(storage, tip_commitment)?;
    verify_tip_parent_header(storage, &tip_block, tip_commitment)?;
    verify_tip_ol_state(storage, tip_commitment)?;
    verify_previous_epoch_summary_for_tip(storage, &tip_block)
}

pub(crate) fn run_startup_checks(ctx: &NodeContext) -> Result<()> {
    let bitcoin_network_check =
        ctx.executor()
            .handle()
            .block_on(run_bitcoin_connectivity_and_network_checks(
                ctx.bitcoin_client().as_ref(),
                ctx.config().bitcoind.network,
            ))?;
    if let StartupBitcoinCheck::Deferred { reason } = bitcoin_network_check {
        warn!(%reason, "startup: deferring Bitcoin RPC network check");
    }

    let latest_client_state = ctx
        .storage()
        .client_state()
        .fetch_most_recent_state()
        .context("startup: failed to fetch most recent client state")?;
    let has_client_state = latest_client_state.is_some();
    let finalized_epoch = latest_client_state
        .as_ref()
        .and_then(|(_, state)| state.get_declared_final_epoch());
    let genesis_commitment = get_ol_genesis_block(ctx.storage().as_ref())?;
    let has_ol_genesis = genesis_commitment.is_some();
    validate_persisted_state_presence(has_client_state, has_ol_genesis)?;

    if has_client_state {
        let l1_anchor_check = ctx.executor().handle().block_on(verify_l1_anchor_block(
            ctx.bitcoin_client().as_ref(),
            ctx.ol_params().last_l1_block,
        ))?;
        if let StartupBitcoinCheck::Deferred { reason } = l1_anchor_check {
            warn!(%reason, "startup: deferring L1 anchor block check");
        }
    }

    let is_sequencer = ctx.config().client.is_sequencer;

    if let Some(genesis_commitment) = genesis_commitment {
        let storage = ctx.storage().as_ref();
        verify_genesis_ol_state(storage, genesis_commitment)?;
        verify_genesis_epoch_summary(storage, genesis_commitment)?;
        ensure_genesis_canonical_entry(storage, genesis_commitment)?;
        let history_base = storage
            .ol_block()
            .get_history_base_blocking()
            .context("startup: failed to query OL history base")?;

        // Only the sequencer backfills the full canonical OL block index. FCM is
        // the sole writer that advances it past genesis and does not run on
        // checkpoint-sync nodes, which store no OL blocks past genesis.
        if is_sequencer {
            if let Some(history_base) = history_base {
                ensure_canonical_block_index_from_history_base(
                    storage,
                    history_base,
                    finalized_epoch,
                )?;
            } else {
                ensure_canonical_block_index(storage, genesis_commitment, finalized_epoch)?;
            }
        }

        let tip_commitment = resolve_tip_ol_block(storage)?;
        if let Some(history_base) = history_base {
            verify_tip_from_history_base(storage, tip_commitment, history_base)?;
        } else {
            let tip_block = verify_tip_ol_block(storage, tip_commitment)?;
            verify_tip_parent(storage, &tip_block, tip_commitment)?;
            verify_tip_ol_state(storage, tip_commitment)?;
            verify_previous_epoch_summary_for_tip(storage, &tip_block)?;
        }
    }

    info!("startup: startup checks passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bitcoin::{BlockHash, Network, Work, hashes::Hash};
    use bitcoind_async_client::corepc_types::model::GetBlockchainInfo;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{MmrId, ol_block::BlockStatus};
    use strata_identifiers::{Buf32, L1BlockId};
    use strata_ledger_types::IStateAccessorMut;
    use strata_ol_params::OLParams;
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::MMR_SENTINEL_DUMMY_LEAF_HASH;
    use strata_storage::{NodeStorage, create_node_storage};

    use super::*;
    use crate::genesis::init_ol_genesis;

    fn make_blockchain_info(network: Network) -> GetBlockchainInfo {
        GetBlockchainInfo {
            chain: network,
            blocks: 100,
            headers: 100,
            best_block_hash: BlockHash::all_zeros(),
            difficulty: 1.0,
            median_time: 600,
            verification_progress: 1.0,
            initial_block_download: false,
            chain_work: Work::from_be_bytes([0; 32]),
            size_on_disk: 1_000_000,
            pruned: false,
            prune_height: None,
            automatic_pruning: None,
            prune_target_size: None,
            bits: None,
            target: None,
            time: None,
            signet_challenge: None,
            warnings: vec![],
            softforks: BTreeMap::new(),
        }
    }

    #[derive(Clone)]
    enum MockResult<T> {
        Ok(T),
        ClientError(ClientError),
    }

    impl<T: Clone> MockResult<T> {
        fn to_result(&self) -> Result<T> {
            match self {
                Self::Ok(value) => Ok(value.clone()),
                Self::ClientError(err) => Err(err.clone().into()),
            }
        }
    }

    struct MockBitcoinClient {
        blockchain_info_result: Option<MockResult<GetBlockchainInfo>>,
        block_hash_result: Option<MockResult<BlockHash>>,
    }

    #[async_trait]
    impl StartupBitcoinClient for MockBitcoinClient {
        async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo> {
            self.blockchain_info_result.as_ref().unwrap().to_result()
        }

        async fn get_block_hash_for_startup(&self, _height: u64) -> Result<BlockHash> {
            self.block_hash_result.as_ref().unwrap().to_result()
        }
    }

    fn mock_client_ok(network: Network) -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: Some(MockResult::Ok(make_blockchain_info(network))),
            block_hash_result: None,
        }
    }

    fn mock_client_unreachable() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: Some(MockResult::ClientError(ClientError::Connection(
                "connection refused".into(),
            ))),
            block_hash_result: None,
        }
    }

    fn mock_client_warming_up() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: Some(MockResult::ClientError(ClientError::Server(
                -28,
                "Loading block index...".into(),
            ))),
            block_hash_result: None,
        }
    }

    fn mock_client_with_block_hash(hash: BlockHash) -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: None,
            block_hash_result: Some(MockResult::Ok(hash)),
        }
    }

    fn mock_client_block_hash_unreachable() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: None,
            block_hash_result: Some(MockResult::ClientError(ClientError::Connection(
                "connection refused".into(),
            ))),
        }
    }

    fn mock_client_block_hash_warming_up() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: None,
            block_hash_result: Some(MockResult::ClientError(ClientError::Server(
                -28,
                "Loading block index...".into(),
            ))),
        }
    }

    fn mock_client_block_hash_height_out_of_range() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: None,
            block_hash_result: Some(MockResult::ClientError(ClientError::Server(
                -8,
                "Block height out of range".into(),
            ))),
        }
    }

    #[tokio::test]
    async fn test_bitcoind_unreachable() {
        let client = mock_client_unreachable();

        let result = run_bitcoin_connectivity_and_network_checks(&client, Network::Regtest).await;

        let check = result.expect("retryable connection failure should defer");
        assert!(matches!(check, StartupBitcoinCheck::Deferred { .. }));
    }

    #[tokio::test]
    async fn test_bitcoind_warmup_deferred() {
        let client = mock_client_warming_up();

        let result = run_bitcoin_connectivity_and_network_checks(&client, Network::Regtest).await;

        let check = result.expect("bitcoind warmup should defer");
        assert!(matches!(check, StartupBitcoinCheck::Deferred { .. }));
    }

    #[test]
    fn test_context_wrapped_bitcoind_warmup_is_retryable() {
        let err = anyhow::Error::from(ClientError::Server(-28, "Loading block index...".into()))
            .context("startup: could not connect to bitcoind via getblockchaininfo");

        assert!(is_retryable_startup_error(&err));
    }

    #[test]
    fn test_context_wrapped_block_height_out_of_range_is_retryable() {
        let err = anyhow::Error::from(ClientError::Server(-8, "Block height out of range".into()))
            .context("startup: failed to fetch L1 block hash from bitcoind");

        assert!(is_retryable_startup_error(&err));
    }

    #[tokio::test]
    async fn test_bitcoind_network_mismatch() {
        let client = mock_client_ok(Network::Bitcoin);

        let result = run_bitcoin_connectivity_and_network_checks(&client, Network::Regtest).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("network mismatch"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_bitcoind_network_matches() {
        let client = mock_client_ok(Network::Regtest);

        let result = run_bitcoin_connectivity_and_network_checks(&client, Network::Regtest).await;

        assert_eq!(result.unwrap(), StartupBitcoinCheck::Verified);
    }

    fn make_l1_block_commitment(height: u32, hash: BlockHash) -> L1BlockCommitment {
        let block_id = hash.to_l1_block_id();
        L1BlockCommitment::new(height, block_id)
    }

    #[tokio::test]
    async fn test_l1_anchor_block_hash_matches() {
        let hash = BlockHash::all_zeros();
        let commitment = make_l1_block_commitment(42, hash);
        let client = mock_client_with_block_hash(hash);

        let result = verify_l1_anchor_block(&client, commitment).await;

        assert_eq!(result.unwrap(), StartupBitcoinCheck::Verified);
    }

    #[tokio::test]
    async fn test_l1_anchor_block_hash_mismatch() {
        let expected_hash = BlockHash::all_zeros();
        let actual_hash = BlockHash::from_byte_array([1; 32]);
        let commitment = make_l1_block_commitment(42, expected_hash);
        let client = mock_client_with_block_hash(actual_hash);

        let result = verify_l1_anchor_block(&client, commitment).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("genesis L1 block hash mismatch"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_l1_anchor_block_unreachable() {
        let hash = BlockHash::all_zeros();
        let commitment = make_l1_block_commitment(42, hash);
        let client = mock_client_block_hash_unreachable();

        let result = verify_l1_anchor_block(&client, commitment).await;

        let check = result.expect("retryable connection failure should defer");
        assert!(matches!(check, StartupBitcoinCheck::Deferred { .. }));
    }

    #[tokio::test]
    async fn test_l1_anchor_block_warmup_deferred() {
        let hash = BlockHash::all_zeros();
        let commitment = make_l1_block_commitment(42, hash);
        let client = mock_client_block_hash_warming_up();

        let result = verify_l1_anchor_block(&client, commitment).await;

        let check = result.expect("bitcoind warmup should defer");
        assert!(matches!(check, StartupBitcoinCheck::Deferred { .. }));
    }

    #[tokio::test]
    async fn test_l1_anchor_block_height_out_of_range_deferred() {
        let hash = BlockHash::all_zeros();
        let commitment = make_l1_block_commitment(42, hash);
        let client = mock_client_block_hash_height_out_of_range();

        let result = verify_l1_anchor_block(&client, commitment).await;

        let check = result.expect("bitcoind missing anchor height should defer");
        assert!(matches!(check, StartupBitcoinCheck::Deferred { .. }));
    }

    fn setup_storage_with_genesis() -> (NodeStorage, OLBlockCommitment) {
        let db = get_test_sled_backend();
        let storage = create_node_storage(db, strata_storage::test_runtime_handle())
            .expect("test: create node storage");
        let genesis_l1_block = L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero()));
        let params = OLParams::new_empty(
            genesis_l1_block,
            strata_ol_params::BridgeParams::new_with_descriptor_limit(
                100_000_000,
                Some(1_000_000_000),
                81,
            )
            .expect("valid bridge params"),
        );
        let genesis_commitment = init_ol_genesis(&params, &storage).expect("test: init ol genesis");
        (storage, genesis_commitment)
    }

    fn setup_storage_with_non_genesis_tip() -> (NodeStorage, OLBlockCommitment, OLBlockCommitment) {
        let (storage, genesis_commitment) = setup_storage_with_genesis();

        let genesis_block = storage
            .ol_block()
            .get_block_data_blocking(*genesis_commitment.blkid())
            .expect("test: query genesis block")
            .expect("test: genesis block exists");

        let mut tip_block = genesis_block.clone();
        tip_block.signed_header.header.slot = 1;
        tip_block.signed_header.header.epoch = 1;
        tip_block.signed_header.header.parent_blkid = *genesis_commitment.blkid();

        let tip_blkid = tip_block.header().compute_blkid();
        let tip_commitment = OLBlockCommitment::new(1, tip_blkid);

        storage
            .ol_block()
            .put_block_data_blocking(tip_block)
            .expect("test: insert tip block");
        storage
            .ol_block()
            .set_block_status_blocking(tip_blkid, BlockStatus::Valid)
            .expect("test: set tip block status");

        let genesis_state = storage
            .ol_state()
            .get_toplevel_ol_state_blocking(genesis_commitment)
            .expect("test: query genesis state")
            .expect("test: genesis state exists");
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(tip_commitment, (*genesis_state).clone())
            .expect("test: insert tip state");

        (storage, genesis_commitment, tip_commitment)
    }

    struct HistoryAnchorFixture {
        storage: NodeStorage,
        genesis_commitment: OLBlockCommitment,
        history_base: EpochCommitment,
        header: OLBlockHeader,
        summary: EpochSummary,
    }

    fn setup_history_anchor_with(
        store_terminal_header: bool,
        is_terminal: bool,
        summary_state_root: Option<Buf32>,
        state_position: Option<(u64, u32)>,
    ) -> HistoryAnchorFixture {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        let genesis_block = storage
            .ol_block()
            .get_block_data_blocking(*genesis_commitment.blkid())
            .expect("test: query genesis block")
            .expect("test: genesis block exists");

        let mut header = genesis_block.header().clone();
        header.slot = 1;
        header.epoch = 1;
        header.parent_blkid = OLBlockId::from(Buf32::from([7; 32]));
        header.flags.set_is_terminal(is_terminal);
        let anchor_commitment = header.compute_block_commitment();
        let history_base = EpochCommitment::new(
            header.epoch(),
            anchor_commitment.slot(),
            *anchor_commitment.blkid(),
        );

        let genesis_epoch_commitment = EpochCommitment::new(0, 0, *genesis_commitment.blkid());
        let genesis_summary = storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(genesis_epoch_commitment)
            .expect("test: query genesis summary")
            .expect("test: genesis summary exists");
        let summary = genesis_summary.create_next_epoch_summary(
            anchor_commitment,
            *genesis_summary.new_l1(),
            summary_state_root.unwrap_or(*header.state_root()),
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .expect("test: insert anchor summary");

        if store_terminal_header {
            storage
                .ol_block()
                .put_terminal_header_blocking(*anchor_commitment.blkid(), header.clone())
                .expect("test: insert anchor terminal header");
        }

        if let Some((state_slot, state_epoch)) = state_position {
            let genesis_state = storage
                .ol_state()
                .get_toplevel_ol_state_blocking(genesis_commitment)
                .expect("test: query genesis state")
                .expect("test: genesis state exists");
            let mut state = MemoryStateBaseLayer::new((*genesis_state).clone());
            state.set_cur_slot(state_slot);
            state.set_cur_epoch(state_epoch);
            storage
                .ol_state()
                .put_toplevel_ol_state_blocking(anchor_commitment, state.into_inner())
                .expect("test: insert anchor state");
        }

        storage
            .ol_block()
            .promote_to_history_anchor_blocking(history_base)
            .expect("test: promote history anchor");

        HistoryAnchorFixture {
            storage,
            genesis_commitment,
            history_base,
            header,
            summary,
        }
    }

    fn setup_valid_history_anchor() -> HistoryAnchorFixture {
        setup_history_anchor_with(true, true, None, Some((1, 2)))
    }

    fn setup_epoch_two_history_anchor(store_previous_epoch_summary: bool) -> HistoryAnchorFixture {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        let genesis_block = storage
            .ol_block()
            .get_block_data_blocking(*genesis_commitment.blkid())
            .expect("test: query genesis block")
            .expect("test: genesis block exists");
        let genesis_epoch_commitment = EpochCommitment::new(0, 0, *genesis_commitment.blkid());
        let genesis_summary = storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(genesis_epoch_commitment)
            .expect("test: query genesis summary")
            .expect("test: genesis summary exists");

        let mut previous_header = genesis_block.header().clone();
        previous_header.slot = 1;
        previous_header.epoch = 1;
        previous_header.parent_blkid = *genesis_commitment.blkid();
        previous_header.flags.set_is_terminal(true);
        let previous_commitment = previous_header.compute_block_commitment();
        let previous_summary = genesis_summary.create_next_epoch_summary(
            previous_commitment,
            *genesis_summary.new_l1(),
            *previous_header.state_root(),
        );
        if store_previous_epoch_summary {
            storage
                .ol_checkpoint()
                .insert_epoch_summary_blocking(previous_summary)
                .expect("test: insert previous epoch summary and canonical index entry");
        }

        let mut header = genesis_block.header().clone();
        header.slot = 2;
        header.epoch = 2;
        header.parent_blkid = *previous_commitment.blkid();
        header.flags.set_is_terminal(true);
        let anchor_commitment = header.compute_block_commitment();
        let history_base = EpochCommitment::from_terminal(header.epoch(), anchor_commitment);
        let summary = previous_summary.create_next_epoch_summary(
            anchor_commitment,
            *previous_summary.new_l1(),
            *header.state_root(),
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .expect("test: insert anchor summary");
        storage
            .ol_block()
            .put_terminal_header_blocking(*anchor_commitment.blkid(), header.clone())
            .expect("test: insert anchor terminal header");

        let genesis_state = storage
            .ol_state()
            .get_toplevel_ol_state_blocking(genesis_commitment)
            .expect("test: query genesis state")
            .expect("test: genesis state exists");
        let mut state = MemoryStateBaseLayer::new((*genesis_state).clone());
        state.set_cur_slot(anchor_commitment.slot());
        state.set_cur_epoch(header.epoch() + 1);
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(anchor_commitment, state.into_inner())
            .expect("test: insert anchor state");
        storage
            .ol_block()
            .promote_to_history_anchor_blocking(history_base)
            .expect("test: promote history anchor");

        HistoryAnchorFixture {
            storage,
            genesis_commitment,
            history_base,
            header,
            summary,
        }
    }

    fn add_tip_above_history_anchor(fixture: &HistoryAnchorFixture) -> OLBlockCommitment {
        let genesis_block = fixture
            .storage
            .ol_block()
            .get_block_data_blocking(*fixture.genesis_commitment.blkid())
            .expect("test: query genesis block")
            .expect("test: genesis block exists");
        let mut tip_block = genesis_block;
        tip_block.signed_header.header.slot = fixture.history_base.last_slot() + 1;
        tip_block.signed_header.header.epoch = fixture.history_base.epoch() + 1;
        tip_block.signed_header.header.parent_blkid = *fixture.history_base.last_blkid();
        tip_block.signed_header.header.flags.set_is_terminal(false);

        let tip_commitment = tip_block.header().compute_block_commitment();
        fixture
            .storage
            .ol_block()
            .put_block_data_blocking(tip_block)
            .expect("test: insert tip above anchor");
        fixture
            .storage
            .ol_block()
            .set_block_status_blocking(*tip_commitment.blkid(), BlockStatus::Valid)
            .expect("test: set tip status");

        let anchor_state = fixture
            .storage
            .ol_state()
            .get_toplevel_ol_state_blocking(fixture.history_base.to_block_commitment())
            .expect("test: query anchor state")
            .expect("test: anchor state exists");
        fixture
            .storage
            .ol_state()
            .put_toplevel_ol_state_blocking(tip_commitment, (*anchor_state).clone())
            .expect("test: insert tip state");
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(
                tip_commitment.slot(),
                vec![*tip_commitment.blkid()],
            )
            .expect("test: advance canonical tip");

        tip_commitment
    }

    #[test]
    fn test_genesis_entries_exist() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();

        let commitment = get_ol_genesis_block(&storage)
            .expect("test: query genesis OL block")
            .expect("test: genesis OL block should exist");
        verify_genesis_ol_state(&storage, commitment).expect("test: genesis OL state should exist");
        verify_genesis_epoch_summary(&storage, commitment)
            .expect("test: genesis epoch summary should exist");

        assert_eq!(commitment, genesis_commitment);
    }

    #[test]
    fn test_missing_canonical_index_is_backfilled_through_finalized_epoch() {
        let (storage, genesis_commitment, tip_commitment) = setup_storage_with_non_genesis_tip();
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(0, Vec::new())
            .expect("test: clear canonical index");

        storage
            .ol_block()
            .get_canonical_tip_blocking()
            .expect_err("test: missing canonical index has no tip");

        let finalized_epoch = EpochCommitment::new(1, 1, *tip_commitment.blkid());
        ensure_canonical_block_index(&storage, genesis_commitment, Some(finalized_epoch))
            .expect("test: backfill canonical index");

        assert_eq!(
            storage
                .ol_block()
                .get_canonical_tip_blocking()
                .expect("test: get canonical tip after backfill"),
            Some(tip_commitment)
        );
    }

    #[test]
    fn test_missing_canonical_index_without_finalized_epoch_backfills_genesis_only() {
        let (storage, genesis_commitment, _) = setup_storage_with_non_genesis_tip();
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(0, Vec::new())
            .expect("test: clear canonical index");

        ensure_canonical_block_index(&storage, genesis_commitment, None)
            .expect("test: backfill canonical index");

        assert_eq!(
            storage
                .ol_block()
                .get_canonical_tip_blocking()
                .expect("test: get canonical tip after backfill"),
            Some(genesis_commitment)
        );
    }

    #[test]
    fn test_checkpoint_sync_seeds_genesis_canonical_entry() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(0, Vec::new())
            .expect("test: clear canonical index");
        storage
            .ol_block()
            .get_canonical_tip_blocking()
            .expect_err("test: missing canonical index has no tip");

        ensure_genesis_canonical_entry(&storage, genesis_commitment)
            .expect("test: seed genesis canonical entry");

        let tip_commitment = resolve_tip_ol_block(&storage).expect("test: resolve tip");
        assert_eq!(tip_commitment, genesis_commitment);

        let tip_block = verify_tip_ol_block(&storage, tip_commitment).expect("test: tip block");
        verify_tip_parent(&storage, &tip_block, tip_commitment).expect("test: tip parent");
        verify_tip_ol_state(&storage, tip_commitment).expect("test: tip state");
        verify_previous_epoch_summary_for_tip(&storage, &tip_block)
            .expect("test: previous epoch summary");
    }

    #[test]
    fn test_ensure_genesis_canonical_entry_is_idempotent() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        // Genesis init already seeded the entry; a second call must be a no-op.
        ensure_genesis_canonical_entry(&storage, genesis_commitment).expect("test: idempotent");
        assert_eq!(
            storage
                .ol_block()
                .get_canonical_tip_blocking()
                .expect("test: canonical tip"),
            Some(genesis_commitment)
        );
    }

    #[test]
    fn test_ensure_genesis_canonical_entry_mismatch_errors() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        let wrong_genesis = OLBlockCommitment::new(0, OLBlockId::from(Buf32::from([9u8; 32])));

        let result = ensure_genesis_canonical_entry(&storage, wrong_genesis);

        let err = result.expect_err("test: mismatch must error");
        assert!(
            err.to_string()
                .contains("canonical OL genesis block mismatch"),
            "unexpected error: {err:#}"
        );
        // The stored genesis entry must be left untouched.
        assert_eq!(
            storage
                .ol_block()
                .get_canonical_block_at_blocking(0)
                .expect("test: query canonical genesis"),
            Some(genesis_commitment)
        );
    }

    #[test]
    fn test_sequencer_missing_finalized_block_errors() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(0, Vec::new())
            .expect("test: clear canonical index");

        let finalized_epoch = EpochCommitment::new(1, 1, OLBlockId::from(Buf32::from([7u8; 32])));
        let result =
            ensure_canonical_block_index(&storage, genesis_commitment, Some(finalized_epoch));

        let err = result.expect_err("test: sequencer must error on missing finalized block");
        assert!(
            format!("{err:#}").contains("missing finalized OL block"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_tip_validation_happy_path_without_parent_record() {
        let fixture = setup_valid_history_anchor();
        let anchor_commitment = fixture.history_base.to_block_commitment();

        assert!(
            fixture
                .storage
                .ol_block()
                .get_block_data_blocking(*anchor_commitment.blkid())
                .expect("test: query anchor block")
                .is_none(),
            "anchor must remain header-only"
        );
        assert!(
            fixture
                .storage
                .ol_block()
                .get_ol_header_blocking(*fixture.header.parent_blkid())
                .expect("test: query absent anchor parent")
                .is_none(),
            "anchor parent must have no record of any kind"
        );
        assert_eq!(
            fixture
                .storage
                .ol_block()
                .get_block_status_blocking(*anchor_commitment.blkid())
                .expect("test: query anchor status"),
            None,
            "startup must not require or fabricate anchor block status"
        );

        verify_tip_from_history_base(&fixture.storage, anchor_commitment, fixture.history_base)
            .expect("test: valid header-only anchor tip");
    }

    #[test]
    fn test_history_anchor_tip_checks_previous_epoch_summary() {
        let fixture = setup_valid_history_anchor();

        verify_previous_epoch_summary_for_tip_header(&fixture.storage, &fixture.header)
            .expect("test: anchor previous epoch summary");
    }

    #[test]
    fn test_epoch_two_history_anchor_with_previous_epoch_summary_passes() {
        let fixture = setup_epoch_two_history_anchor(true);

        verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect("test: valid epoch-two history anchor");
        verify_previous_epoch_summary_for_tip_header(&fixture.storage, &fixture.header)
            .expect("test: epoch-one summary and canonical entry must be present");
        assert!(
            fixture
                .storage
                .ol_checkpoint()
                .get_canonical_epoch_commitment_at_blocking(1)
                .expect("test: query previous canonical epoch")
                .is_some()
        );
    }

    #[test]
    fn test_epoch_two_history_anchor_missing_previous_epoch_summary_fails_distinctly() {
        let fixture = setup_epoch_two_history_anchor(false);

        verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect("test: anchor validation should pass independently");
        let err = verify_previous_epoch_summary_for_tip_header(&fixture.storage, &fixture.header)
            .expect_err("test: missing epoch-one summary must fail");

        assert_eq!(
            err.to_string(),
            "startup: missing epoch summary for previous epoch"
        );
    }

    #[test]
    fn test_history_anchor_missing_terminal_header_fails_distinctly() {
        let fixture = setup_history_anchor_with(false, true, None, Some((1, 2)));

        let err = verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect_err("test: missing terminal header must fail");
        assert!(
            err.to_string()
                .contains("missing OL history anchor terminal header"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_non_terminal_header_fails_distinctly() {
        let fixture = setup_history_anchor_with(true, false, None, Some((1, 2)));

        let err = verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect_err("test: non-terminal anchor must fail");
        assert!(
            err.to_string()
                .contains("OL history anchor header is not terminal"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_header_summary_blkid_mismatch_fails_distinctly() {
        let fixture = setup_valid_history_anchor();
        let mut mismatched_header = fixture.header.clone();
        mismatched_header.timestamp += 1;

        let err = verify_anchor_header_summary(
            fixture.history_base,
            &mismatched_header,
            &fixture.summary,
        )
        .expect_err("test: header/summary block ID mismatch must fail");
        assert!(
            err.to_string()
                .contains("OL history anchor header/summary block ID mismatch"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_header_slot_mismatch_fails_distinctly() {
        let fixture = setup_valid_history_anchor();
        let mut mismatched_header = fixture.header.clone();
        mismatched_header.slot += 1;

        let err = verify_anchor_header_summary(
            fixture.history_base,
            &mismatched_header,
            &fixture.summary,
        )
        .expect_err("test: header slot mismatch must fail");
        assert!(
            err.to_string()
                .contains("OL history anchor header slot mismatch"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_header_epoch_mismatch_fails_distinctly() {
        let fixture = setup_valid_history_anchor();
        let mut mismatched_header = fixture.header.clone();
        mismatched_header.epoch += 1;

        let err = verify_anchor_header_summary(
            fixture.history_base,
            &mismatched_header,
            &fixture.summary,
        )
        .expect_err("test: header epoch mismatch must fail");
        assert!(
            err.to_string()
                .contains("OL history anchor header epoch mismatch"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_state_root_mismatch_fails_distinctly() {
        let fixture =
            setup_history_anchor_with(true, true, Some(Buf32::from([9; 32])), Some((1, 2)));

        let err = verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect_err("test: anchor state root mismatch must fail");
        assert!(
            err.to_string()
                .contains("OL history anchor state root mismatch"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_missing_state_fails_distinctly() {
        let fixture = setup_history_anchor_with(true, true, None, None);

        let err = verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect_err("test: missing anchor state must fail");
        assert!(
            err.to_string().contains("missing OL history anchor state"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_state_epoch_mismatch_fails_distinctly() {
        let fixture = setup_history_anchor_with(true, true, None, Some((1, 1)));

        let err = verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect_err("test: anchor state epoch mismatch must fail");
        assert!(
            err.to_string()
                .contains("OL history anchor state epoch mismatch: expected 2, got 1"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_history_anchor_state_slot_mismatch_fails_distinctly() {
        let fixture = setup_history_anchor_with(true, true, None, Some((2, 2)));

        let err = verify_history_anchor(&fixture.storage, fixture.history_base)
            .expect_err("test: anchor state slot mismatch must fail");
        assert!(
            err.to_string()
                .contains("OL history anchor state slot mismatch: expected 1, got 2"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_tip_above_history_anchor_accepts_header_only_parent() {
        let fixture = setup_valid_history_anchor();
        let tip_commitment = add_tip_above_history_anchor(&fixture);

        verify_tip_from_history_base(&fixture.storage, tip_commitment, fixture.history_base)
            .expect("test: tip above header-only anchor");
    }

    #[test]
    fn test_tip_above_history_anchor_missing_parent_fails_as_before() {
        let fixture = setup_history_anchor_with(false, true, None, Some((1, 2)));
        let tip_commitment = add_tip_above_history_anchor(&fixture);

        let err =
            verify_tip_from_history_base(&fixture.storage, tip_commitment, fixture.history_base)
                .expect_err("test: missing tip parent must fail");
        assert_eq!(
            err.to_string(),
            "startup: missing OL parent block for non-genesis tip"
        );
    }

    #[test]
    fn test_finalized_chain_walk_stops_at_history_base_without_older_blocks() {
        let fixture = setup_valid_history_anchor();
        let tip_commitment = add_tip_above_history_anchor(&fixture);
        fixture
            .storage
            .ol_block()
            .del_block_data_blocking(*fixture.genesis_commitment.blkid())
            .expect("test: delete block below history base");

        let chain = collect_finalized_canonical_chain_from_history_base(
            &fixture.storage,
            fixture.history_base,
            tip_commitment,
        )
        .expect("test: collect finalized chain from history base");

        assert_eq!(
            chain,
            vec![*fixture.history_base.last_blkid(), *tip_commitment.blkid()]
        );
    }

    #[test]
    fn test_finalized_chain_walk_missing_block_above_history_base_fails_as_before() {
        let fixture = setup_valid_history_anchor();
        let missing_commitment = OLBlockCommitment::new(
            fixture.history_base.last_slot() + 1,
            OLBlockId::from(Buf32::from([8; 32])),
        );

        let err = collect_finalized_canonical_chain_from_history_base(
            &fixture.storage,
            fixture.history_base,
            missing_commitment,
        )
        .expect_err("test: missing finalized block above base must fail");
        assert!(
            err.to_string()
                .contains("startup: missing finalized OL block"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn test_l1_block_refs_mmr_prefilled_at_ol_genesis() {
        let (storage, _) = setup_storage_with_genesis();
        let handle = storage.mmr_index().get_handle(MmrId::L1BlockRefs);

        assert_eq!(
            handle
                .get_num_leaves_blocking()
                .expect("test: get leaf count"),
            1
        );
        assert_eq!(
            handle.get_leaf_blocking(0).expect("test: get leaf"),
            Some(MMR_SENTINEL_DUMMY_LEAF_HASH)
        );
    }

    #[test]
    fn test_genesis_entries_missing() {
        let db = get_test_sled_backend();
        let storage = create_node_storage(db, strata_storage::test_runtime_handle())
            .expect("test: create node storage");

        let commitment = get_ol_genesis_block(&storage).expect("test: query genesis OL block");
        assert!(commitment.is_none());
    }

    #[test]
    fn test_persisted_state_presence_is_consistent_when_both_present() {
        let result = validate_persisted_state_presence(true, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_persisted_state_presence_is_consistent_when_both_absent() {
        let result = validate_persisted_state_presence(false, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_client_state_without_ol_genesis_fails() {
        let result = validate_persisted_state_presence(true, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_ol_genesis_without_client_state_fails() {
        let result = validate_persisted_state_presence(false, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_tip_entries_exist_after_two_post_genesis_blocks() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();

        let genesis_block = storage
            .ol_block()
            .get_block_data_blocking(*genesis_commitment.blkid())
            .expect("test: query genesis block")
            .expect("test: genesis block exists");
        let genesis_state = storage
            .ol_state()
            .get_toplevel_ol_state_blocking(genesis_commitment)
            .expect("test: query genesis state")
            .expect("test: genesis state exists");

        // Add block 1 (slot 1, epoch 1) with genesis as parent.
        let mut block_1 = genesis_block.clone();
        block_1.signed_header.header.slot = 1;
        block_1.signed_header.header.epoch = 1;
        block_1.signed_header.header.parent_blkid = *genesis_commitment.blkid();

        let block_1_blkid = block_1.header().compute_blkid();
        let block_1_commitment = OLBlockCommitment::new(1, block_1_blkid);

        storage
            .ol_block()
            .put_block_data_blocking(block_1.clone())
            .expect("test: insert block 1");
        storage
            .ol_block()
            .set_block_status_blocking(block_1_blkid, BlockStatus::Valid)
            .expect("test: set block 1 status");
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(block_1_commitment, (*genesis_state).clone())
            .expect("test: insert block 1 state");
        // Update the canonical index.
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(1, vec![block_1_blkid])
            .expect("test: canonical entry for slot 1");

        // Validate checks with tip at block 1.
        let result_after_block_1 = (|| {
            let tip_commitment = resolve_tip_ol_block(&storage)?;
            let tip_block = verify_tip_ol_block(&storage, tip_commitment)?;
            verify_tip_parent(&storage, &tip_block, tip_commitment)?;
            verify_tip_ol_state(&storage, tip_commitment)?;
            verify_previous_epoch_summary_for_tip(&storage, &tip_block)?;
            Ok::<_, anyhow::Error>(tip_commitment)
        })();

        assert_eq!(
            result_after_block_1.expect("tip checks after block 1"),
            block_1_commitment
        );

        // Add epoch 1 summary (derived from genesis summary + block 1 terminal).
        let genesis_epoch_commitment = EpochCommitment::new(0, 0, *genesis_commitment.blkid());
        let genesis_summary = storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(genesis_epoch_commitment)
            .expect("test: query genesis summary")
            .expect("test: genesis summary exists");
        let epoch_1_summary = genesis_summary.create_next_epoch_summary(
            block_1_commitment,
            *genesis_summary.new_l1(),
            *block_1.header().state_root(),
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(epoch_1_summary)
            .expect("test: insert epoch 1 summary");

        // Add block 2 (slot 2, epoch 2) with block 1 as parent.
        let mut block_2 = block_1.clone();
        block_2.signed_header.header.slot = 2;
        block_2.signed_header.header.epoch = 2;
        block_2.signed_header.header.parent_blkid = block_1_blkid;

        let block_2_blkid = block_2.header().compute_blkid();
        let block_2_commitment = OLBlockCommitment::new(2, block_2_blkid);

        storage
            .ol_block()
            .put_block_data_blocking(block_2)
            .expect("test: insert block 2");
        storage
            .ol_block()
            .set_block_status_blocking(block_2_blkid, BlockStatus::Valid)
            .expect("test: set block 2 status");
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(block_2_commitment, (*genesis_state).clone())
            .expect("test: insert block 2 state");
        // Update the canonical index.
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(2, vec![block_2_blkid])
            .expect("test: canonical entry for slot 2");

        // Validate checks with tip at block 2.
        let result_after_block_2 = (|| {
            let tip_commitment = resolve_tip_ol_block(&storage)?;
            let tip_block = verify_tip_ol_block(&storage, tip_commitment)?;
            verify_tip_parent(&storage, &tip_block, tip_commitment)?;
            verify_tip_ol_state(&storage, tip_commitment)?;
            verify_previous_epoch_summary_for_tip(&storage, &tip_block)?;
            Ok::<_, anyhow::Error>(tip_commitment)
        })();

        assert_eq!(
            result_after_block_2.expect("tip checks after block 2"),
            block_2_commitment
        );
    }

    #[test]
    fn test_tip_parent_missing_fails() {
        let (storage, genesis_commitment, tip_commitment) = setup_storage_with_non_genesis_tip();
        storage
            .ol_block()
            .del_block_data_blocking(*genesis_commitment.blkid())
            .expect("test: delete parent block");

        let result = (|| {
            let tip_block = verify_tip_ol_block(&storage, tip_commitment)?;
            verify_tip_parent(&storage, &tip_block, tip_commitment)
        })();

        assert!(result.is_err());
    }

    #[test]
    fn test_tip_state_missing_fails() {
        let (storage, _, tip_commitment) = setup_storage_with_non_genesis_tip();
        storage
            .ol_state()
            .del_toplevel_ol_state_blocking(tip_commitment)
            .expect("test: delete tip state");

        let result = verify_tip_ol_state(&storage, tip_commitment);

        assert!(result.is_err());
    }

    #[test]
    fn test_previous_epoch_summary_missing_fails() {
        let (storage, genesis_commitment, tip_commitment) = setup_storage_with_non_genesis_tip();
        let epoch_commitment = EpochCommitment::new(0, 0, *genesis_commitment.blkid());
        storage
            .ol_checkpoint()
            .del_epoch_summary_blocking(epoch_commitment)
            .expect("test: delete previous epoch summary");

        let result = (|| {
            let tip_block = verify_tip_ol_block(&storage, tip_commitment)?;
            verify_previous_epoch_summary_for_tip(&storage, &tip_block)
        })();

        assert!(result.is_err());
    }
}
