use std::sync::Arc;

use bitcoin::{key::UntweakedKeypair, secp256k1::SECP256K1, Address};
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::WriterConfig;
use strata_status::StatusChannel;

use crate::BtcioParams;

/// All the items that writer tasks need as context.
#[derive(Debug, Clone)]
pub(crate) struct WriterContext<R: Reader + Signer + Wallet> {
    /// Btcio required parameters
    pub btcio_params: BtcioParams,

    /// Btcio specific configuration.
    pub config: Arc<WriterConfig>,

    /// Sequencer's address to watch utxos for and spend change amount to.
    pub sequencer_address: Address,

    /// Bitcoin client to sign and submit transactions.
    pub client: Arc<R>,

    /// Channel for receiving latest states.
    pub status_channel: StatusChannel,

    /// Optional sequencer keypair for SPS-51 envelope authentication.
    ///
    /// When set, this keypair is used as the taproot key in envelope transactions,
    /// allowing the ASM to verify the envelope was created by the sequencer by
    /// checking the pubkey against the sequencer predicate.
    pub envelope_keypair: Option<UntweakedKeypair>,
}

impl<R: Reader + Signer + Wallet> WriterContext<R> {
    pub(crate) fn new(
        btcio_params: BtcioParams,
        config: Arc<WriterConfig>,
        sequencer_address: Address,
        client: Arc<R>,
        status_channel: StatusChannel,
    ) -> Self {
        Self {
            btcio_params,
            config,
            sequencer_address,
            client,
            status_channel,
            envelope_keypair: None,
        }
    }

    /// Sets the sequencer keypair from raw secret key bytes.
    ///
    /// The keypair will be used as the taproot key in envelope transactions
    /// for SPS-51 authentication.
    pub(crate) fn with_sequencer_sk(mut self, sk_bytes: &[u8; 32]) -> Self {
        let keypair = UntweakedKeypair::from_seckey_slice(SECP256K1, sk_bytes)
            .expect("valid secret key bytes");
        self.envelope_keypair = Some(keypair);
        self
    }
}
