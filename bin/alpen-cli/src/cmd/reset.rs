use argh::FromArgs;
use colored::Colorize;
use dialoguer::Confirm;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{seed::EncryptedSeedPersister, settings::Settings};

/// DANGER: resets the CLI completely, destroying all keys and databases.
/// Keeps config.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "reset")]
pub struct ResetArgs {
    /// dangerous: permit to reset without further confirmation
    #[argh(switch, short = 'y')]
    assume_yes: bool,
}

pub async fn reset(
    args: ResetArgs,
    persister: impl EncryptedSeedPersister,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let confirm = if args.assume_yes {
        true
    } else {
        println!("{}", "This will DESTROY ALL DATA.".to_string().red().bold());
        Confirm::new()
            .with_prompt("Do you REALLY want to continue?")
            .interact()
            .internal_error("Failed to read user confirmation")?
    };

    if confirm {
        persister
            .delete()
            .internal_error("Failed to wipe out seed")?;
        println!("Wiped seed");
        std::fs::remove_dir_all(settings.data_dir.clone())
            .internal_error("Failed to delete data directory")?;
        println!("Wiped data directory");
    }

    Ok(())
}
