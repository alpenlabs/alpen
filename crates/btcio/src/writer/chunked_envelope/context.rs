use std::sync::Arc;

use bitcoin::Address;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::WriterConfig;

use crate::BtcioParams;

/// All the items that chunked writer tasks need as context.
#[derive(Debug, Clone)]
pub(crate) struct ChunkedWriterContext<R: Reader + Signer + Wallet> {
    /// Btcio-specific parameters.
    pub btcio_params: BtcioParams,

    /// Btcio specific configuration.
    pub config: Arc<WriterConfig>,

    /// Sequencer's address to watch utxos for and spend change amount to.
    pub sequencer_address: Address,

    /// Bitcoin client to sign and submit transactions.
    pub client: Arc<R>,
}

impl<R: Reader + Signer + Wallet> ChunkedWriterContext<R> {
    pub(crate) fn new(
        btcio_params: BtcioParams,
        config: Arc<WriterConfig>,
        sequencer_address: Address,
        client: Arc<R>,
    ) -> Self {
        Self {
            btcio_params,
            config,
            sequencer_address,
            client,
        }
    }
}
