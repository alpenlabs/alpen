//! Alpen protocol spec activations.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

/// Identifier of an Alpen protocol spec revision.
///
/// The set of specs is closed: only variants defined here are valid keys in an
/// [`AlpenSpecActivations`], so an unknown or misspelled spec name fails to
/// decode instead of being accepted as an activation the node cannot honor.
///
/// The enum is currently uninhabited, so `BTreeMap<AlpenSpecId, _>` is
/// provably empty — exactly the placeholder invariant, enforced by the type
/// system rather than a runtime check.
// TODO(STR-3997): define the real spec variants and what each resolves to per
// component (e.g. revm spec id, program VKs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlpenSpecId {}

impl fmt::Display for AlpenSpecId {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Uninhabited today; the match becomes exhaustive once real variants
        // exist.
        match *self {}
    }
}

/// Placeholder Alpen spec activations.
///
/// Maps [`AlpenSpecId`]s to activation values so the params artifact already
/// carries the `spec_activations` axis in its schema. No specs are defined,
/// so the map stays empty by construction.
// TODO(STR-3997): refine the activation value type (height vs. timestamp) with the
// real activations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AlpenSpecActivations(BTreeMap<AlpenSpecId, u64>);

impl AlpenSpecActivations {
    /// Returns whether no spec activations are scheduled.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::AlpenSpecActivations;

    #[test]
    fn empty_map_deserializes() {
        let activations: AlpenSpecActivations =
            serde_json::from_str("{}").expect("empty map is valid");
        assert!(activations.is_empty());
    }

    #[test]
    fn unknown_spec_id_is_rejected() {
        // No `AlpenSpecId` variant exists yet, so any key fails to decode.
        assert!(serde_json::from_str::<AlpenSpecActivations>(r#"{"upgrade":100}"#).is_err());
    }
}
