//! Checkpoint predicate resolution based on enabled features and CLI overrides.

use std::{error, fmt, str::FromStr};

use strata_predicate::PredicateKey;

/// CLI override for the checkpoint predicate type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckpointPredicateOverride {
    /// Force `AlwaysAccept` regardless of compile-time features.
    AlwaysAccept,
    /// Use SP1 Groth16 (requires `sp1-builder` feature).
    Sp1Groth16,
}

/// Error returned when parsing a [`CheckpointPredicateOverride`] from a CLI string.
#[derive(Debug)]
pub(crate) struct ParseCheckpointPredicateError(String);

impl fmt::Display for ParseCheckpointPredicateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid checkpoint predicate type '{}', expected 'always-accept' or 'sp1-groth16'",
            self.0
        )
    }
}

impl error::Error for ParseCheckpointPredicateError {}

impl FromStr for CheckpointPredicateOverride {
    type Err = ParseCheckpointPredicateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "always-accept" => Ok(Self::AlwaysAccept),
            "sp1-groth16" => Ok(Self::Sp1Groth16),
            _ => Err(ParseCheckpointPredicateError(s.to_owned())),
        }
    }
}

/// Returns the appropriate [`PredicateKey`] based on the optional CLI override
/// or the enabled compile-time features.
///
/// If `override_val` is `Some`, it takes precedence over feature flags.
/// Otherwise falls back to the feature-gated default.
pub(crate) fn resolve_checkpoint_predicate(
    override_val: Option<CheckpointPredicateOverride>,
) -> anyhow::Result<PredicateKey> {
    match override_val {
        Some(CheckpointPredicateOverride::AlwaysAccept) => Ok(PredicateKey::always_accept()),
        Some(CheckpointPredicateOverride::Sp1Groth16) => resolve_sp1_groth16(),
        None => Ok(resolve_default()),
    }
}

/// Resolves the SP1 Groth16 predicate key.
///
/// Returns an error if the `sp1-builder` feature is not enabled.
fn resolve_sp1_groth16() -> anyhow::Result<PredicateKey> {
    #[cfg(feature = "sp1-builder")]
    {
        Ok(build_sp1_predicate())
    }

    #[cfg(not(feature = "sp1-builder"))]
    {
        anyhow::bail!(
            "--checkpoint-predicate sp1-groth16 requires the binary to be built with \
             -F sp1-builder"
        );
    }
}

/// Returns the feature-gated default predicate.
fn resolve_default() -> PredicateKey {
    #[cfg(feature = "sp1-builder")]
    {
        build_sp1_predicate()
    }

    #[cfg(not(feature = "sp1-builder"))]
    {
        PredicateKey::always_accept()
    }
}

#[cfg(feature = "sp1-builder")]
fn build_sp1_predicate() -> PredicateKey {
    use strata_predicate::PredicateTypeId;
    use strata_primitives::buf::Buf32;
    use strata_sp1_guest_builder::GUEST_CHECKPOINT_VK_HASH_STR;
    use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;

    let vk_buf32: Buf32 = GUEST_CHECKPOINT_VK_HASH_STR
        .parse()
        .expect("invalid sp1 checkpoint verifier key hash");
    let sp1_verifier = SP1Groth16Verifier::load(&sp1_verifier::GROTH16_VK_BYTES, vk_buf32.0)
        .expect("Failed to load SP1 Groth16 verifier");
    let condition_bytes = sp1_verifier.vk.to_uncompressed_bytes();
    PredicateKey::new(PredicateTypeId::Sp1Groth16, condition_bytes)
}
