//! Runtime checkpoint authentication state derived from ASM.

use std::{fmt, sync::Arc};

use bitcoin::secp256k1::{Error as Secp256k1Error, XOnlyPublicKey};
use strata_asm_common::Subprotocol;
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_btcio::writer::{EnvelopeSigningMode, EnvelopeSigningModeProvider};
use strata_db_types::errors::DbError;
use strata_predicate::PredicateTypeId;
use strata_storage::NodeStorage;

/// Errors produced while resolving checkpoint envelope authentication.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CheckpointAuthError {
    /// The canonical ASM state could not be loaded from storage.
    #[error("failed to fetch canonical ASM state")]
    FetchCanonicalAsmState(#[source] DbError),

    /// No canonical ASM state exists yet.
    #[error("canonical ASM state is not available")]
    MissingCanonicalAsmState,

    /// The checkpoint subprotocol section is missing from ASM state.
    #[error("canonical ASM state is missing checkpoint subprotocol state")]
    MissingCheckpointState,

    /// The checkpoint subprotocol section could not be decoded.
    #[error("failed to decode checkpoint subprotocol state")]
    DecodeCheckpointState(#[source] strata_asm_common::AsmError),

    /// The checkpoint sequencer predicate type is unknown.
    #[error("unknown checkpoint sequencer predicate {0}")]
    UnknownPredicate(u8),

    /// The checkpoint sequencer predicate has an invalid condition length.
    #[error("Bip340Schnorr checkpoint sequencer predicate has {0} condition bytes")]
    InvalidSchnorrConditionLength(usize),

    /// The checkpoint sequencer predicate condition is not a valid x-only key.
    #[error("invalid checkpoint sequencer x-only pubkey")]
    InvalidSchnorrPubkey(#[source] Secp256k1Error),

    /// The active checkpoint sequencer predicate cannot sign envelopes.
    #[error("checkpoint sequencer predicate is NeverAccept")]
    NeverAccept,

    /// The active checkpoint sequencer predicate is not supported for envelope authentication.
    #[error("checkpoint sequencer predicate cannot use Sp1Groth16")]
    Sp1Groth16,
}

/// Resolves the active checkpoint sequencer key from the canonical ASM state.
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
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }

    fn resolve_signing_mode(&self) -> Result<EnvelopeSigningMode, CheckpointAuthError> {
        let (_, asm_state) = self
            .storage
            .fetch_canonical_asm_state_blocking()
            .map_err(CheckpointAuthError::FetchCanonicalAsmState)?
            .ok_or(CheckpointAuthError::MissingCanonicalAsmState)?;

        let checkpoint_section = asm_state
            .state()
            .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
            .ok_or(CheckpointAuthError::MissingCheckpointState)?;

        let checkpoint_state = checkpoint_section
            .try_to_state::<CheckpointSubprotocol>()
            .map_err(CheckpointAuthError::DecodeCheckpointState)?;

        let predicate = checkpoint_state.sequencer_predicate();
        let predicate_id = predicate.id();
        let predicate_type = PredicateTypeId::try_from(predicate_id)
            .map_err(|_| CheckpointAuthError::UnknownPredicate(predicate_id))?;

        match predicate_type {
            PredicateTypeId::AlwaysAccept => Ok(EnvelopeSigningMode::InProcess),
            PredicateTypeId::Bip340Schnorr => {
                let pubkey_bytes: [u8; 32] = predicate.condition().try_into().map_err(|_| {
                    CheckpointAuthError::InvalidSchnorrConditionLength(predicate.condition().len())
                })?;
                let pubkey = XOnlyPublicKey::from_slice(&pubkey_bytes)
                    .map_err(CheckpointAuthError::InvalidSchnorrPubkey)?;
                Ok(EnvelopeSigningMode::External { pubkey })
            }
            PredicateTypeId::NeverAccept => Err(CheckpointAuthError::NeverAccept),
            PredicateTypeId::Sp1Groth16 => Err(CheckpointAuthError::Sp1Groth16),
        }
    }
}

impl EnvelopeSigningModeProvider for CheckpointSequencerKeyProvider {
    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
        self.resolve_signing_mode().map_err(Into::into)
    }
}
