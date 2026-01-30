use std::path::PathBuf;

use argh::FromArgs;

/// Strata sequencer signer args.
#[derive(Debug, FromArgs, Clone)]
pub(crate) struct Args {
    /// path to sequencer root key
    #[argh(option, short = 'k')]
    pub sequencer_key: Option<PathBuf>,

    /// JSON-RPC host
    #[argh(option, short = 'h')]
    pub rpc_host: Option<String>,

    /// JSON-RPC port
    #[argh(option, short = 'r')]
    pub rpc_port: Option<u16>,

    /// duty polling interval in milliseconds
    #[argh(option, short = 'p')]
    pub duty_poll_interval: Option<u64>,
}
