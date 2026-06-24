use std::{fmt::Debug, sync::Arc};

use bitcoin::{secp256k1::XOnlyPublicKey, Address};
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::WriterConfig;
use strata_status::StatusChannel;

use crate::BtcioParams;

/// How the writer should authenticate the next envelope transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvelopeSigningMode {
    /// Builds and signs with a temporary key in-process.
    InProcess,
    /// Builds an envelope for the configured external signer pubkey.
    External { pubkey: XOnlyPublicKey },
}

/// Resolves the envelope signing mode for the current canonical state.
pub trait EnvelopeSigningModeProvider: Send + Sync + Debug + 'static {
    /// Returns the signing mode to use for the next envelope.
    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode>;
}

/// Static signing mode provider used by tests and simple configurations.
#[derive(Debug)]
struct StaticEnvelopeSigningModeProvider {
    mode: EnvelopeSigningMode,
}

impl StaticEnvelopeSigningModeProvider {
    fn new(mode: EnvelopeSigningMode) -> Self {
        Self { mode }
    }
}

impl EnvelopeSigningModeProvider for StaticEnvelopeSigningModeProvider {
    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
        Ok(self.mode)
    }
}

/// All the items that writer tasks need as context.
#[derive(Debug, Clone)]
pub struct WriterContext<R: Reader + Signer + Wallet> {
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

    /// Source for the current SPS-51 envelope authentication mode.
    signing_mode_provider: Arc<dyn EnvelopeSigningModeProvider>,
}

impl<R: Reader + Signer + Wallet> WriterContext<R> {
    pub fn new(
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
            signing_mode_provider: Arc::new(StaticEnvelopeSigningModeProvider::new(
                EnvelopeSigningMode::InProcess,
            )),
        }
    }

    /// Sets the sequencer public key from raw 32-byte x-only pubkey bytes.
    ///
    /// The pubkey will be used as the taproot key in envelope transactions
    /// for SPS-51 authentication. Signing is handled externally by the signer binary.
    pub fn with_envelope_pubkey(mut self, pubkey_bytes: &[u8; 32]) -> Self {
        let pubkey =
            XOnlyPublicKey::from_slice(pubkey_bytes).expect("valid x-only public key bytes");
        self.signing_mode_provider = Arc::new(StaticEnvelopeSigningModeProvider::new(
            EnvelopeSigningMode::External { pubkey },
        ));
        self
    }

    /// Sets a dynamic provider for SPS-51 envelope authentication.
    pub fn with_signing_mode_provider(
        mut self,
        provider: Arc<dyn EnvelopeSigningModeProvider>,
    ) -> Self {
        self.signing_mode_provider = provider;
        self
    }

    /// Returns the current envelope signing mode.
    pub fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
        self.signing_mode_provider.signing_mode()
    }
}
