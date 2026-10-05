//! Trusted native ASM execution parameters distributed with the network parameters.

use serde::{Deserialize, Serialize};
use strata_asm_common::SpecId;
use strata_predicate::PredicateKey;

/// Initial ASM authority and trusted predicate-to-implementation associations.
///
/// Load these from the network's execution-parameters JSON, independently of the
/// proving backend. Keep associations immutable across restarts. Upstream native
/// assembly validates the catalog; it does not authenticate these associations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsmExecutionParams {
    genesis_predicate: PredicateKey,
    targets: Vec<AsmExecutionTarget>,
}

impl AsmExecutionParams {
    /// Assembles parameters for validation by the upstream native ASM registry.
    pub fn new(genesis_predicate: PredicateKey, targets: Vec<AsmExecutionTarget>) -> Self {
        Self {
            genesis_predicate,
            targets,
        }
    }

    /// Returns the predicate authorizing the chain's initial ASM ruleset.
    pub fn genesis_predicate(&self) -> &PredicateKey {
        &self.genesis_predicate
    }

    /// Returns the trusted native execution catalog.
    pub fn targets(&self) -> &[AsmExecutionTarget] {
        &self.targets
    }
}

/// A trusted association between an ASM predicate and a compiled implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsmExecutionTarget {
    predicate: PredicateKey,
    spec_id: SpecId,
}

impl AsmExecutionTarget {
    /// Associates a program identity with a native implementation.
    pub fn new(predicate: PredicateKey, spec_id: SpecId) -> Self {
        Self { predicate, spec_id }
    }

    /// Returns the program identity associated with this implementation.
    pub fn predicate(&self) -> &PredicateKey {
        &self.predicate
    }

    /// Returns the protocol spec ID of the compiled implementation.
    pub fn spec_id(&self) -> SpecId {
        self.spec_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_params_preserve_json_fields() {
        let json = serde_json::json!({
            "genesis_predicate": "AlwaysAccept",
            "targets": [{"predicate": "AlwaysAccept", "spec_id": 0}]
        });
        let params: AsmExecutionParams = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            params,
            AsmExecutionParams::new(
                PredicateKey::always_accept(),
                vec![AsmExecutionTarget::new(PredicateKey::always_accept(), 0)],
            )
        );
        assert_eq!(params.genesis_predicate(), &PredicateKey::always_accept());
        assert_eq!(
            params.targets()[0].predicate(),
            &PredicateKey::always_accept()
        );
        assert_eq!(params.targets()[0].spec_id(), 0);
        assert_eq!(serde_json::to_value(params).unwrap(), json);
    }
}
