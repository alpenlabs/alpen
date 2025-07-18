use argh::FromArgs;

use crate::{
    errors::{DisplayableError, DisplayedError},
    recovery::DescriptorRecovery,
    seed::Seed,
    settings::Settings,
};

/// Various debug utilities
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "debug")]
pub struct DebugArgs {
    #[argh(subcommand)]
    pub cmd: DebugCommands,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub enum DebugCommands {
    Recovery(RecoveryArgs),
}

/// Handler for individual debug commands
pub async fn debug(args: DebugArgs, seed: Seed, settings: Settings) -> Result<(), DisplayedError> {
    match args.cmd {
        DebugCommands::Recovery(args) => recovery(args, seed, settings).await,
    }
}

/// Debug utilities for descriptor recovery
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "recovery")]
pub struct RecoveryArgs {}

/// Handler for descriptor recovery debug, just decrypting and showing the contents of the recovery
/// database at the moment.
pub async fn recovery(
    _args: RecoveryArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let mut descriptor_file = DescriptorRecovery::open(&seed, &settings.descriptor_db)
        .await
        .internal_error("Failed to open descriptor recovery file")?;

    let descs = descriptor_file
        .read_descs(..)
        .await
        .internal_error("Failed to read descriptors after chain height")?;

    dbg!(descs);

    Ok(())
}
