//! Account predicate resolution for the Alpen EE snark account.

use std::{error, fmt, str::FromStr};

use strata_predicate::PredicateKey;
use strata_proofimpl_alpen_acct::EeAcctProgram;

/// CLI override for the account predicate type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcctPredicateOverride {
    AlwaysAccept,
    Sp1Groth16,
    /// Use BIP-340 Schnorr bound to the alpen-acct program's deterministic
    /// test signing key (functional tests).
    Bip340SchnorrTest,
}

#[derive(Debug)]
pub(crate) struct ParseAcctPredicateError(String);

impl fmt::Display for ParseAcctPredicateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid account predicate type '{}', expected 'always-accept', 'sp1-groth16', or 'bip340-schnorr-test'",
            self.0
        )
    }
}

impl error::Error for ParseAcctPredicateError {}

impl FromStr for AcctPredicateOverride {
    type Err = ParseAcctPredicateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "always-accept" => Ok(Self::AlwaysAccept),
            "sp1-groth16" => Ok(Self::Sp1Groth16),
            "bip340-schnorr-test" => Ok(Self::Bip340SchnorrTest),
            _ => Err(ParseAcctPredicateError(s.to_owned())),
        }
    }
}

pub(crate) fn resolve_acct_predicate(
    override_val: Option<AcctPredicateOverride>,
) -> anyhow::Result<PredicateKey> {
    match override_val {
        Some(AcctPredicateOverride::AlwaysAccept) => Ok(PredicateKey::always_accept()),
        Some(AcctPredicateOverride::Sp1Groth16) => resolve_sp1_groth16(),
        Some(AcctPredicateOverride::Bip340SchnorrTest) => Ok(EeAcctProgram::test_predicate_key()),
        None => Ok(resolve_default()),
    }
}

fn resolve_sp1_groth16() -> anyhow::Result<PredicateKey> {
    #[cfg(feature = "sp1-builder")]
    {
        Ok(build_sp1_predicate())
    }

    #[cfg(not(feature = "sp1-builder"))]
    {
        anyhow::bail!(
            "--alpen-predicate sp1-groth16 requires the binary to be built with -F sp1-builder"
        );
    }
}

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
    use strata_primitives::buf::Buf32;
    use strata_proofimpl_predicate_keys::{PredicateKeyProvider, Sp1Groth16PredicateKey};
    use strata_sp1_guest_builder::GUEST_ALPEN_ACCT_VK_HASH_STR;

    let vk_buf32: Buf32 = GUEST_ALPEN_ACCT_VK_HASH_STR
        .parse()
        .expect("invalid sp1 alpen-acct verifier key hash");
    Sp1Groth16PredicateKey::new(vk_buf32.0)
        .predicate_key()
        .expect("failed to build SP1 Groth16 account predicate")
}
