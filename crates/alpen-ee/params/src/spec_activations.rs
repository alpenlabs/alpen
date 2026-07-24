//! Alpen protocol spec activations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Placeholder Alpen spec activations.
///
/// Maps spec names to activation values so the params artifact already
/// carries the `spec_activations` axis in its schema. The real activation
/// types (spec-id enum, resolver) land with STR-3997 and replace the value
/// type; until then the map is expected to stay empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AlpenSpecActivations(BTreeMap<String, u64>);

impl AlpenSpecActivations {
    /// Returns whether no spec activations are scheduled.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
