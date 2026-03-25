use std::sync::Arc;

use bitcoin::{secp256k1::XOnlyPublicKey, Address};
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

    /// Optional sequencer public key for SPS-51 envelope authentication.
    ///
    /// When set, this pubkey is used as the taproot key in envelope transactions,
    /// allowing the ASM to verify the envelope was created by the sequencer by
    /// checking the pubkey against the sequencer predicate. The actual signing
    /// is done externally by the strata-signer binary.
    pub envelope_pubkey: Option<XOnlyPublicKey>,
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
            envelope_pubkey: None,
        }
    }

    /// Sets the sequencer public key from raw 32-byte x-only pubkey bytes.
    ///
    /// The pubkey will be used as the taproot key in envelope transactions
    /// for SPS-51 authentication. Signing is handled externally by the signer binary.
    pub(crate) fn with_envelope_pubkey(mut self, pubkey_bytes: &[u8; 32]) -> Self {
        let pubkey =
            XOnlyPublicKey::from_slice(pubkey_bytes).expect("valid x-only public key bytes");
        self.envelope_pubkey = Some(pubkey);
        self
    }
}
