use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{bitcoin::BitcoinWallet, seed::Seed, settings::Settings};

/// Performs a full scan of the Bitcoin wallet
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "scan")]
pub struct ScanArgs {}

pub async fn scan(_args: ScanArgs, seed: Seed, settings: Settings) -> Result<(), DisplayedError> {
    let mut l1w = BitcoinWallet::new(&seed, settings.network, settings.bitcoin_backend.clone())
        .internal_error("Failed to load Bitcoin wallet")?;
    l1w.scan()
        .await
        .internal_error("Failed to scan Bitcoin wallet")?;

    Ok(())
}
