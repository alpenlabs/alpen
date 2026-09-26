//! Chainstate layouts and the spec-to-layout mapping.

use std::fmt;

use strata_ol_state_types::OLSpecId;

/// Identifies a chainstate layout, one variant of
/// [`OLStateSeries`](crate::OLStateSeries).
///
/// Each layout is the toplevel state type of one `strata-ol-state-types-v*`
/// crate, so a new layout means a new such crate. Layouts have no numeric
/// value and are never serialized; the root's current spec determines them
/// through [`Self::for_spec`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum OLStateLayout {
    /// [`OLStateV1`](strata_ol_state_types_v1::OLStateV1), from
    /// `strata-ol-state-types-v1`.
    V1,
}

impl OLStateLayout {
    /// Returns the layout of a chainstate produced under `spec`.
    ///
    /// This is the only spec-to-layout mapping. A spec that keeps the previous
    /// layout adds an arm naming it; a spec with a new layout adds a layout
    /// variant too.
    pub fn for_spec(spec: OLSpecId) -> Self {
        match spec {
            OLSpecId::V1 => Self::V1,
        }
    }
}

impl fmt::Display for OLStateLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V1 => f.write_str("V1"),
        }
    }
}
