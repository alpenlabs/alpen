//! Alpen protocol fork schedule.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Placeholder Alpen fork schedule.
///
/// Maps fork names to activation values so the params artifact already
/// carries the `alpen_forks` axis in its schema. The real schedule types
/// (hardfork enum, spec-id resolver) land with STR-3997 and replace the
/// value type; until then the map is expected to stay empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AlpenForkSchedule(BTreeMap<String, u64>);

impl AlpenForkSchedule {
    /// Returns whether no fork activations are scheduled.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
