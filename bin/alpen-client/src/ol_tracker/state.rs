use std::sync::Arc;

use strata_acct_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::OLBlockCommitment;
use tracing::warn;

use crate::{config::AlpenEeConfig, traits::storage::Storage};

#[derive(Debug)]
pub(crate) struct OlTrackerState {
    pub(crate) ee_state: EeAccountState,
    pub(crate) ol_block: OLBlockCommitment,
}

pub(crate) async fn init_ol_tracker_state<TStorage>(
    config: Arc<AlpenEeConfig>,
    storage: Arc<TStorage>,
) -> eyre::Result<OlTrackerState>
where
    TStorage: Storage,
{
    let best_state = match storage
        .best_ee_account_state()
        .await
        .map_err(|e| eyre::eyre!(e))?
    {
        Some(s) => OlTrackerState {
            ee_state: s.state,
            ol_block: s.ol_block,
        },
        None => {
            // initialize using genesis config
            warn!("ee state not found; create using genesis config");
            let genesis_state = EeAccountState::new(
                *config.params.genesis_blockhash.as_ref(),
                BitcoinAmount::zero(),
                vec![],
                vec![],
            );
            let genesis_ol_block = OLBlockCommitment::new(
                config.params.genesis_ol_slot,
                config.params.genesis_ol_blockid,
            );
            // persist genesis state
            storage
                .store_ee_account_state(&genesis_ol_block, &genesis_state)
                .await
                .map_err(|e| eyre::eyre!(e))?;

            OlTrackerState {
                ee_state: genesis_state,
                ol_block: genesis_ol_block,
            }
        }
    };

    Ok(best_state)
}
