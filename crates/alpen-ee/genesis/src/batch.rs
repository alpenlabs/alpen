//! For handling deterministic genesis blocks used in EE.

use alpen_ee_common::{Batch, BatchStorage};
use alpen_ee_config::AlpenEeConfig;
use eyre::{eyre, Context};

pub async fn ensure_batch_genesis<TStorage: BatchStorage>(
    config: &AlpenEeConfig,
    storage: &TStorage,
) -> eyre::Result<()> {
    let expected_genesis_batch = Batch::new_genesis_batch(
        config.params().genesis_blockhash().0.into(),
        config.params().genesis_blocknum(),
    )
    .map_err(|err| eyre!("ensure_batch_genesis: {err}"))?;

    if let Some((stored_genesis_batch, _)) = storage
        .get_batch_by_idx(0)
        .await
        .map_err(eyre::Error::from)
        .context("ensure_batch_genesis: failed to get genesis batch")?
    {
        if stored_genesis_batch != expected_genesis_batch {
            return Err(eyre::eyre!(
                "ensure_batch_genesis: unexpected genesis batch found in storage"
            ));
        }
    }

    // If exists, does not overwrite
    storage
        .save_genesis_batch(expected_genesis_batch)
        .await
        .map_err(eyre::Error::from)
        .context("ensure_batch_genesis: failed to create genesis batch")?;

    Ok(())
}
