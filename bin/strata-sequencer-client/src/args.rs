use std::path::PathBuf;

use argh::FromArgs;

#[derive(Debug, Clone, FromArgs)]
#[argh(description = "Alpen Strata sequencer")]
pub(crate) struct Args {
    #[argh(option, short = 'k', description = "path to sequencer root key")]
    pub sequencer_key: Option<PathBuf>,

    #[argh(option, short = 'h', description = "JSON-RPC host")]
    pub rpc_host: Option<String>,

    #[argh(option, short = 'r', description = "JSON-RPC port")]
    pub rpc_port: Option<u16>,

    #[argh(option, short = 'i', description = "poll interval for duties in ms")]
    pub duty_poll_interval: Option<u64>,

    #[argh(
        option,
        short = 'l',
        description = "evm gas limit per epoch (optional)"
    )]
    pub epoch_gas_limit: Option<u64>,
}
