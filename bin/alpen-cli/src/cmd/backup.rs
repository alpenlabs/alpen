use argh::FromArgs;
use bip39::Language;
use strata_cli_common::errors::DisplayedError;

use crate::seed::Seed;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backup")]
/// Prints a BIP39 mnemonic encoding the internal wallet's seed bytes
pub struct BackupArgs {}

pub async fn backup(_args: BackupArgs, seed: Seed) -> Result<(), DisplayedError> {
    seed.print_mnemonic(Language::English);
    Ok(())
}
