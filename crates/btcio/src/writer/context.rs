use std::sync::Arc;

use bitcoin::Address;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::WriterConfig;
use strata_primitives::params::Params;
use strata_status::StatusChannel;

/// All the items that writer tasks need as context.
#[derive(Debug, Clone)]
pub struct WriterContext<R: Reader + Signer + Wallet> {
    /// Params for rollup.
    pub params: Arc<Params>,

    /// Btcio specific configuration.
    pub config: Arc<WriterConfig>,

    /// Sequencer's address to watch utxos for and spend change amount to.
    pub sequencer_address: Address,

    /// Bitcoin client to sign and submit transactions.
    pub client: Arc<R>,

    /// Channel for receiving latest states.
    pub status_channel: StatusChannel,
}

impl<R: Reader + Signer + Wallet> WriterContext<R> {
    pub fn new(
        params: Arc<Params>,
        config: Arc<WriterConfig>,
        sequencer_address: Address,
        client: Arc<R>,
        status_channel: StatusChannel,
    ) -> Self {
        Self {
            params,
            config,
            sequencer_address,
            client,
            status_channel,
        }
    }
}
