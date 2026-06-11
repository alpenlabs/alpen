//! Predicate-key providers for Alpen proof programs.

use strata_predicate::PredicateKey;
use strata_proofimpl_alpen_acct::EeAcctProgram;
use strata_proofimpl_alpen_chunk::EeChunkProgram;
use strata_proofimpl_checkpoint::program::CheckpointProgram;

/// Errors returned while deriving or validating predicate keys.
#[derive(Debug, thiserror::Error)]
pub enum PredicateKeyError {
    /// Failed to build an SP1 Groth16 verifier predicate condition.
    #[cfg(feature = "sp1")]
    #[error("failed to derive SP1 Groth16 predicate key: {0}")]
    Sp1Verifier(String),

    /// The requested provider is unavailable because a crate feature is disabled.
    #[error("{0}")]
    FeatureDisabled(&'static str),

    /// The configured predicate does not match the expected predicate.
    #[error(
        "predicate key mismatch: configured type {configured_type} condition length \
         {configured_condition_len}, expected type {expected_type} condition length \
         {expected_condition_len}"
    )]
    Mismatch {
        /// Predicate type configured in params.
        configured_type: u8,
        /// Configured predicate condition byte length.
        configured_condition_len: usize,
        /// Expected predicate type for the selected provider.
        expected_type: u8,
        /// Expected predicate condition byte length.
        expected_condition_len: usize,
    },
}

/// Provides the predicate key for a concrete proof program/backend pairing.
pub trait PredicateKeyProvider {
    /// Returns the predicate key expected for this provider.
    fn predicate_key(&self) -> Result<PredicateKey, PredicateKeyError>;
}

/// Validates that a configured predicate matches a provider-derived predicate.
pub fn validate_predicate_key(
    configured: &PredicateKey,
    provider: &impl PredicateKeyProvider,
) -> Result<(), PredicateKeyError> {
    let expected = provider.predicate_key()?;
    validate_expected_predicate_key(configured, &expected)
}

/// Validates that a configured predicate matches an expected predicate.
pub fn validate_expected_predicate_key(
    configured: &PredicateKey,
    expected: &PredicateKey,
) -> Result<(), PredicateKeyError> {
    if configured.id() == expected.id() && configured.condition() == expected.condition() {
        return Ok(());
    }

    Err(PredicateKeyError::Mismatch {
        configured_type: configured.id(),
        configured_condition_len: configured.condition().len(),
        expected_type: expected.id(),
        expected_condition_len: expected.condition().len(),
    })
}

/// Native checkpoint predicate provider used by functional-test setups.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeCheckpointPredicateKey;

impl PredicateKeyProvider for NativeCheckpointPredicateKey {
    fn predicate_key(&self) -> Result<PredicateKey, PredicateKeyError> {
        Ok(CheckpointProgram::test_predicate_key())
    }
}

/// Native Alpen EE chunk predicate provider used by functional-test setups.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeAlpenChunkPredicateKey;

impl PredicateKeyProvider for NativeAlpenChunkPredicateKey {
    fn predicate_key(&self) -> Result<PredicateKey, PredicateKeyError> {
        Ok(EeChunkProgram::test_predicate_key())
    }
}

/// Native Alpen EE account predicate provider used by functional-test setups.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeAlpenAcctPredicateKey;

impl PredicateKeyProvider for NativeAlpenAcctPredicateKey {
    fn predicate_key(&self) -> Result<PredicateKey, PredicateKeyError> {
        Ok(EeAcctProgram::test_predicate_key())
    }
}

/// SP1 Groth16 predicate provider for a concrete program ID.
#[derive(Debug, Clone, Copy)]
pub struct Sp1Groth16PredicateKey {
    program_id: [u8; 32],
}

impl Sp1Groth16PredicateKey {
    /// Creates an SP1 Groth16 predicate provider bound to a program ID.
    pub fn new(program_id: [u8; 32]) -> Self {
        Self { program_id }
    }
}

#[cfg(feature = "sp1")]
impl PredicateKeyProvider for Sp1Groth16PredicateKey {
    fn predicate_key(&self) -> Result<PredicateKey, PredicateKeyError> {
        use strata_predicate::PredicateTypeId;
        use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;

        let sp1_verifier = SP1Groth16Verifier::load(
            &sp1_verifier::GROTH16_VK_BYTES,
            self.program_id,
            *sp1_verifier::VK_ROOT_BYTES,
            true,
        )
        .map_err(|e| PredicateKeyError::Sp1Verifier(e.to_string()))?;
        let condition = sp1_verifier.to_uncompressed_bytes();

        Ok(PredicateKey::new(PredicateTypeId::Sp1Groth16, condition))
    }
}

#[cfg(not(feature = "sp1"))]
impl PredicateKeyProvider for Sp1Groth16PredicateKey {
    fn predicate_key(&self) -> Result<PredicateKey, PredicateKeyError> {
        let _ = self.program_id;
        Err(PredicateKeyError::FeatureDisabled(
            "SP1 predicate-key derivation requires the `sp1` feature",
        ))
    }
}
