//! Predicate-key providers for Alpen proof programs.

use strata_predicate::PredicateKey;
use strata_proofimpl_alpen_acct::EeAcctProgram;
use strata_proofimpl_alpen_chunk::EeChunkProgram;

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
         {configured_condition_len} condition {configured_condition}, expected type \
         {expected_type} condition length {expected_condition_len} condition \
         {expected_condition}"
    )]
    Mismatch {
        /// Predicate type configured in params.
        configured_type: u8,
        /// Configured predicate condition byte length.
        configured_condition_len: usize,
        /// Hex-encoded configured predicate condition.
        configured_condition: String,
        /// Expected predicate type for the selected provider.
        expected_type: u8,
        /// Expected predicate condition byte length.
        expected_condition_len: usize,
        /// Hex-encoded expected predicate condition.
        expected_condition: String,
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
        configured_condition: hex::encode(configured.condition()),
        expected_type: expected.id(),
        expected_condition_len: expected.condition().len(),
        expected_condition: hex::encode(expected.condition()),
    })
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

#[cfg(test)]
mod tests {
    use strata_predicate::{PredicateKey, PredicateTypeId};

    use super::{
        PredicateKeyError, PredicateKeyProvider, validate_expected_predicate_key,
        validate_predicate_key,
    };

    #[derive(Debug)]
    struct StaticPredicateKeyProvider(PredicateKey);

    impl PredicateKeyProvider for StaticPredicateKeyProvider {
        fn predicate_key(&self) -> Result<PredicateKey, PredicateKeyError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn accepts_equal_predicate_keys() {
        let predicate = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![1, 2, 3]);

        validate_expected_predicate_key(&predicate, &predicate).unwrap();
    }

    #[test]
    fn validates_predicate_key_provider_output() {
        let predicate = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![1, 2, 3]);
        let provider = StaticPredicateKeyProvider(predicate.clone());

        validate_predicate_key(&predicate, &provider).unwrap();
    }

    #[test]
    fn mismatch_reports_type_length_and_conditions() {
        let configured = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![0xaa; 32]);
        let expected = PredicateKey::new(PredicateTypeId::Sp1Groth16, vec![0xbb; 16]);

        let err = validate_expected_predicate_key(&configured, &expected).unwrap_err();

        let PredicateKeyError::Mismatch {
            configured_type,
            configured_condition_len,
            configured_condition,
            expected_type,
            expected_condition_len,
            expected_condition,
        } = err
        else {
            panic!("expected mismatch error");
        };

        assert_eq!(configured_type, PredicateTypeId::Bip340Schnorr as u8);
        assert_eq!(configured_condition_len, 32);
        assert_eq!(
            configured_condition,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(expected_type, PredicateTypeId::Sp1Groth16 as u8);
        assert_eq!(expected_condition_len, 16);
        assert_eq!(expected_condition, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    }

    #[test]
    fn mismatch_reports_same_type_and_length_with_different_conditions() {
        let configured = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![0xaa; 32]);
        let expected = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![0xbb; 32]);

        let err = validate_expected_predicate_key(&configured, &expected)
            .unwrap_err()
            .to_string();

        assert!(err.contains(
            "configured type 10 condition length 32 condition \
             aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(err.contains(
            "expected type 10 condition length 32 condition \
             bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ));
    }
}
