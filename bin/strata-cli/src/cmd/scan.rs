use argh::FromArgs;

use crate::{
    errors::{DisplayableError, DisplayedError},
    seed::Seed,
    settings::Settings,
    signet::SignetWallet,
};

/// Performs a full scan of the signet wallet
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "scan")]
pub struct ScanArgs {}

pub async fn scan(_args: ScanArgs, seed: Seed, settings: Settings) -> Result<(), DisplayedError> {
    let mut l1w = SignetWallet::new(&seed, settings.network, settings.signet_backend.clone())
        .internal_error("Failed to load signet wallet")?;
    l1w.scan()
        .await
        .internal_error("Failed to scan signet wallet")?;

    Ok(())
}
