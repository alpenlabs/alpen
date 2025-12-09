//! For handling deterministic genesis blocks used in EE.

use std::sync::Arc;

use alpen_ee_common::Storage;
use alpen_ee_config::AlpenEeConfig;
use eyre::eyre;
use tracing::{error, warn};

use crate::{build_ee_genesis_ol_epoch, build_genesis_ee_account_state};

pub async fn ensure_genesis_ee_account_state<TStorage: Storage>(
    config: Arc<AlpenEeConfig>,
    storage: Arc<TStorage>,
) -> eyre::Result<()> {
    let genesis_ol_epoch = build_ee_genesis_ol_epoch(config.params());
    let genesis_state = build_genesis_ee_account_state(config.params());

    if let Some(stored_genesis_state) = storage
        .ee_account_state(genesis_ol_epoch.last_blkid().into())
        .await?
    {
        if stored_genesis_state.ee_state() != &genesis_state {
            error!(expected = ?genesis_state, found = ?stored_genesis_state.ee_state(), "unexpected genesis state");
            return Err(eyre!("unexpected genesis state in storage"));
        }
    }

    warn!("ee state not found; create using genesis config");

    // persist genesis state
    storage
        .store_ee_account_state(&genesis_ol_epoch, &genesis_state)
        .await?;

    Ok(())
}
