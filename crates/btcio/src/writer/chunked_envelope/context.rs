use std::sync::Arc;

use bitcoin::{key::Keypair, Address};
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::WriterConfig;

use crate::BtcioParams;

/// All the items that chunked writer tasks need as context.
///
/// The `sequencer_keypair` is the BIP340 Schnorr keypair used to sign reveal
/// txs under `<sequencer_pk> OP_CHECKSIG`.
#[derive(Debug, Clone)]
pub(crate) struct ChunkedWriterContext<R: Reader + Signer + Wallet> {
    /// Btcio-specific parameters.
    pub btcio_params: BtcioParams,

    /// Btcio specific configuration.
    pub config: Arc<WriterConfig>,

    /// Sequencer's address to watch utxos for and spend change amount to.
    pub sequencer_address: Address,

    /// keypair for signing reveal tapscript spends.
    pub sequencer_keypair: Keypair,

    /// Bitcoin client to sign and submit transactions.
    pub client: Arc<R>,
}

impl<R: Reader + Signer + Wallet> ChunkedWriterContext<R> {
    pub(crate) fn new(
        btcio_params: BtcioParams,
        config: Arc<WriterConfig>,
        sequencer_address: Address,
        sequencer_keypair: Keypair,
        client: Arc<R>,
    ) -> Self {
        Self {
            btcio_params,
            config,
            sequencer_address,
            sequencer_keypair,
            client,
        }
    }
}
