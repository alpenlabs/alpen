use std::sync::Arc;

use bitcoin::Address;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::WriterConfig;
use strata_primitives::{l1::payload::L1Payload, params::Params};
use strata_status::StatusChannel;

/// Callable that produces SPS-50 tag bytes for the reveal transaction OP_RETURN.
pub type EnvelopeTagEncoder = dyn Fn(&L1Payload) -> anyhow::Result<Vec<u8>> + Send + Sync;

/// All the items that writer tasks need as context.
#[derive(Clone)]
pub(crate) struct WriterContext<R: Reader + Signer + Wallet> {
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

    /// Function to obtain SPS-50 tag bytes for a given payload.
    pub tag_encoder: Arc<EnvelopeTagEncoder>,
}

impl<R: Reader + Signer + Wallet> WriterContext<R> {
    pub(crate) fn new(
        params: Arc<Params>,
        config: Arc<WriterConfig>,
        sequencer_address: Address,
        client: Arc<R>,
        status_channel: StatusChannel,
        tag_encoder: Arc<EnvelopeTagEncoder>,
    ) -> Self {
        Self {
            params,
            config,
            sequencer_address,
            client,
            status_channel,
            tag_encoder,
        }
    }

    pub(crate) fn encode_tag(&self, payload: &L1Payload) -> anyhow::Result<Vec<u8>> {
        (self.tag_encoder)(payload)
    }
}
