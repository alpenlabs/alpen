use std::{fmt::Debug, sync::Arc};

use bitcoin::{secp256k1::XOnlyPublicKey, Address};
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_config::btcio::WriterConfig;
use strata_csm_types::L1Payload;
use strata_identifiers::{Buf32, Epoch};
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

/// What a queued writer payload is, as far as checkpoint policy is concerned.
///
/// The writer queue carries payloads for every subprotocol and this crate does not
/// know how any of them are encoded, so classification is delegated to a
/// [`CheckpointPayloadInspector`]. The three cases are kept apart because they call
/// for different handling: a payload that is not a checkpoint is none of the
/// writer's business, while one that claims to be a checkpoint and will not decode
/// is worth a warning before the writer falls back to publishing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadCheckpointRef {
    /// Not a checkpoint payload.
    NotCheckpoint,
    /// Tagged as a checkpoint, but the body could not be decoded.
    Undecodable,
    /// A checkpoint payload the inspector could identify.
    Checkpoint {
        /// Epoch the checkpoint attests to.
        epoch: Epoch,
        /// Identity of the checkpoint body, distinguishing candidates within an epoch.
        id: Buf32,
    },
}

/// Identifies the checkpoint a queued writer payload carries, if any.
///
/// Implemented above this crate by whoever owns the checkpoint encoding; btcio only
/// compares the epochs it hands back.
pub trait CheckpointPayloadInspector: Send + Sync + Debug + 'static {
    /// Classifies a payload from the writer queue.
    fn inspect_payload(&self, payload: &L1Payload) -> PayloadCheckpointRef;
}

/// Handles checkpoint-specific cleanup after a retiring envelope fails.
///
/// The writer owns intent and bundle state, but the checkpoint signing marker
/// lives above this crate. Implementations clear that marker only when it still
/// represents the failed checkpoint candidate.
pub trait CheckpointFailureHandler: Send + Sync + Debug + 'static {
    /// Cleans up upper-layer state for a failed retiring checkpoint.
    fn handle_failed_checkpoint(&self, checkpoint: PayloadCheckpointRef) -> anyhow::Result<()>;
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

    /// Identifies the checkpoint behind a queued payload for the watcher's stale gate.
    checkpoint_inspector: Arc<dyn CheckpointPayloadInspector>,

    /// Cleans up checkpoint state when a retiring envelope fails.
    checkpoint_failure_handler: Arc<dyn CheckpointFailureHandler>,
}

impl<R: Reader + Signer + Wallet> WriterContext<R> {
    /// Builds the writer context.
    ///
    /// `checkpoint_inspector` is taken up front rather than through a builder method
    /// with a permissive default: an inspector that recognizes nothing silently
    /// disables the watcher's stale-checkpoint gate, which is exactly the failure the
    /// gate exists to prevent.
    pub fn new(
        btcio_params: BtcioParams,
        config: Arc<WriterConfig>,
        sequencer_address: Address,
        client: Arc<R>,
        status_channel: StatusChannel,
        checkpoint_inspector: Arc<dyn CheckpointPayloadInspector>,
        checkpoint_failure_handler: Arc<dyn CheckpointFailureHandler>,
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
            checkpoint_inspector,
            checkpoint_failure_handler,
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

    /// Classifies a queued payload for the watcher's stale-checkpoint gate.
    pub fn inspect_payload(&self, payload: &L1Payload) -> PayloadCheckpointRef {
        self.checkpoint_inspector.inspect_payload(payload)
    }

    /// Cleans up checkpoint state after a retiring envelope fails.
    pub fn handle_failed_checkpoint(&self, checkpoint: PayloadCheckpointRef) -> anyhow::Result<()> {
        self.checkpoint_failure_handler
            .handle_failed_checkpoint(checkpoint)
    }
}
