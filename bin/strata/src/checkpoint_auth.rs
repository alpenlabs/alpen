//! Runtime checkpoint authentication state derived from ASM.

use std::{fmt, sync::Arc};

use anyhow::{Context, anyhow};
use bitcoin::secp256k1::XOnlyPublicKey;
use strata_asm_common::Subprotocol;
use strata_asm_proto_checkpoint::subprotocol::CheckpointSubprotocol;
use strata_btcio::writer::{EnvelopeSigningMode, EnvelopeSigningModeProvider};
use strata_identifiers::Buf32;
use strata_predicate::PredicateTypeId;
use strata_storage::NodeStorage;

/// Resolves the active checkpoint sequencer key from the latest ASM state.
#[derive(Clone)]
pub(crate) struct CheckpointSequencerKeyProvider {
    storage: Arc<NodeStorage>,
}

impl fmt::Debug for CheckpointSequencerKeyProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckpointSequencerKeyProvider")
            .finish_non_exhaustive()
    }
}

impl CheckpointSequencerKeyProvider {
    /// Creates a provider backed by node storage.
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }

    /// Returns the active external signer pubkey, if one is required.
    pub(crate) fn current_pubkey(&self) -> anyhow::Result<Option<Buf32>> {
        match self.signing_mode()? {
            EnvelopeSigningMode::InProcess => Ok(None),
            EnvelopeSigningMode::External { pubkey } => Ok(Some(Buf32(pubkey.serialize()))),
        }
    }
}

impl EnvelopeSigningModeProvider for CheckpointSequencerKeyProvider {
    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
        let (_, asm_state) = self
            .storage
            .asm()
            .fetch_most_recent_state()
            .context("failed to fetch latest ASM state")?
            .context("latest ASM state is not available")?;

        let checkpoint_section = asm_state
            .state()
            .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
            .context("latest ASM state is missing checkpoint subprotocol state")?;

        let checkpoint_state = checkpoint_section
            .try_to_state::<CheckpointSubprotocol>()
            .context("failed to decode checkpoint subprotocol state")?;

        let predicate = checkpoint_state.sequencer_predicate();
        let predicate_type = PredicateTypeId::try_from(predicate.id()).with_context(|| {
            format!("unknown checkpoint sequencer predicate {}", predicate.id())
        })?;

        match predicate_type {
            PredicateTypeId::AlwaysAccept => Ok(EnvelopeSigningMode::InProcess),
            PredicateTypeId::Bip340Schnorr => {
                let pubkey_bytes: [u8; 32] = predicate.condition().try_into().map_err(|_| {
                    anyhow!(
                        "Bip340Schnorr checkpoint sequencer predicate has {} condition bytes",
                        predicate.condition().len()
                    )
                })?;
                let pubkey = XOnlyPublicKey::from_slice(&pubkey_bytes)
                    .context("invalid checkpoint sequencer x-only pubkey")?;
                Ok(EnvelopeSigningMode::External { pubkey })
            }
            PredicateTypeId::NeverAccept => {
                Err(anyhow!("checkpoint sequencer predicate is NeverAccept"))
            }
            PredicateTypeId::Sp1Groth16 => Err(anyhow!(
                "checkpoint sequencer predicate cannot use Sp1Groth16"
            )),
        }
    }
}
