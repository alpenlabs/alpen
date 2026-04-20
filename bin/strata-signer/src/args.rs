//! CLI argument definitions.

use std::path::PathBuf;

use argh::FromArgs;

#[derive(Debug, FromArgs)]
#[argh(description = "Standalone sequencer signer for Strata")]
pub(crate) struct Args {
    /// path to the TOML configuration file
    #[argh(option, short = 'c')]
    pub(crate) config: PathBuf,
}
