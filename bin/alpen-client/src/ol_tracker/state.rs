use std::sync::Arc;

use strata_acct_types::{BitcoinAmount, Hash};
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::OLBlockCommitment;
use tracing::warn;

use super::error::{OlTrackerError, Result};
use crate::{
    config::AlpenEeConfig,
    traits::{
        ol_client::OlChainStatus,
        storage::{EeAccountStateAtBlock, Storage},
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsensusHeads {
    pub(crate) confirmed: Hash,
    pub(crate) finalized: Hash,
}

impl ConsensusHeads {
    pub(crate) fn confirmed(&self) -> &Hash {
        &self.confirmed
    }

    pub(crate) fn finalized(&self) -> &Hash {
        &self.finalized
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OlTrackerState {
    best: EeAccountStateAtBlock,
    confirmed: EeAccountStateAtBlock,
    finalized: EeAccountStateAtBlock,
}

#[cfg(test)]
impl OlTrackerState {
    pub(crate) fn new(
        best: EeAccountStateAtBlock,
        confirmed: EeAccountStateAtBlock,
        finalized: EeAccountStateAtBlock,
    ) -> Self {
        Self {
            best,
            confirmed,
            finalized,
        }
    }
}

impl OlTrackerState {
    pub(crate) fn best_ee_state(&self) -> &EeAccountState {
        self.best.ee_state()
    }

    pub(crate) fn best_ol_block(&self) -> &OLBlockCommitment {
        self.best.ol_block()
    }

    pub(crate) fn get_consensus_heads(&self) -> ConsensusHeads {
        ConsensusHeads {
            confirmed: self.confirmed.last_exec_blkid(),
            finalized: self.finalized.last_exec_blkid(),
        }
    }
}

/// Initialized [`OlTrackerState`] from storage
pub(crate) async fn init_ol_tracker_state<TStorage>(
    config: Arc<AlpenEeConfig>,
    ol_chain_status: OlChainStatus,
    storage: Arc<TStorage>,
) -> Result<OlTrackerState>
where
    TStorage: Storage,
{
    let Some(best_state) = storage.best_ee_account_state().await? else {
        // nothing in storage, so initialize using genesis config

        warn!("ee state not found; create using genesis config");
        let genesis_state = EeAccountState::new(
            *config.params().genesis_blockhash().as_ref(),
            BitcoinAmount::zero(),
            vec![],
            vec![],
        );
        let genesis_ol_block = OLBlockCommitment::new(
            config.params().genesis_ol_slot(),
            config.params().genesis_ol_blockid(),
        );
        // persist genesis state
        storage
            .store_ee_account_state(&genesis_ol_block, &genesis_state)
            .await?;

        let block_account_state = EeAccountStateAtBlock::new(genesis_ol_block, genesis_state);

        return Ok(OlTrackerState {
            best: block_account_state.clone(),
            confirmed: block_account_state.clone(),
            finalized: block_account_state,
        });
    };

    build_tracker_state(best_state, &ol_chain_status, storage.as_ref()).await
}

pub(crate) async fn build_tracker_state(
    best_state: EeAccountStateAtBlock,
    ol_chain_status: &OlChainStatus,
    storage: &impl Storage,
) -> Result<OlTrackerState> {
    // determine confirmed, finalized states
    let confirmed_state =
        effective_account_state(best_state.ol_block(), ol_chain_status.confirmed(), storage)
            .await
            .map_err(|e| OlTrackerError::BuildStateFailed(format!("confirmed state: {}", e)))?;

    let finalized_state =
        effective_account_state(best_state.ol_block(), ol_chain_status.finalized(), storage)
            .await
            .map_err(|e| OlTrackerError::BuildStateFailed(format!("finalized state: {}", e)))?;

    Ok(OlTrackerState {
        best: best_state,
        confirmed: confirmed_state,
        finalized: finalized_state,
    })
}

pub(crate) async fn effective_account_state(
    local: &OLBlockCommitment,
    ol: &OLBlockCommitment,
    storage: &impl Storage,
) -> Result<EeAccountStateAtBlock> {
    let min_blockid = if local.slot() < ol.slot() {
        local.blkid()
    } else {
        ol.blkid()
    };

    storage
        .ee_account_state(min_blockid.into())
        .await?
        .ok_or_else(|| OlTrackerError::MissingBlock {
            block_id: min_blockid.to_string(),
        })
}
