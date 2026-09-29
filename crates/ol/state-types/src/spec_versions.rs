//! The spec versions an OL state carries.

use thiserror::Error;

use crate::spec_id::{OLSpecId, UnknownOLSpecId};

/// Spec versions of an OL state: the spec its chainstate was produced under and
/// the raw spec version the next epoch runs under.
///
/// A V0 state's root is its bare chainstate root, which commits no staged
/// spec. This type therefore holds a V0 current spec only with V0 staged, so no
/// uncommitted value can vary under one root. From V1 on, the root commits
/// both versions and the staged version may name a spec this binary does not
/// know.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct OLSpecVersions {
    cur_spec: OLSpecId,
    staged_spec_version: u32,
}

impl OLSpecVersions {
    /// Creates spec versions from the current spec and a raw staged version.
    ///
    /// # Errors
    ///
    /// Returns [`NonCanonicalV0Versions`] if `cur_spec` is V0 and
    /// `staged_spec_version` is not.
    pub fn new(
        cur_spec: OLSpecId,
        staged_spec_version: u32,
    ) -> Result<Self, NonCanonicalV0Versions> {
        if cur_spec == OLSpecId::V0 && staged_spec_version != u32::from(OLSpecId::V0) {
            return Err(NonCanonicalV0Versions {
                staged_spec_version,
            });
        }
        Ok(Self {
            cur_spec,
            staged_spec_version,
        })
    }

    /// Creates spec versions whose current and staged spec are both `spec`.
    pub fn uniform(spec: OLSpecId) -> Self {
        Self {
            cur_spec: spec,
            staged_spec_version: spec.into(),
        }
    }

    /// Returns the spec the chainstate was produced under.
    pub fn cur_spec(&self) -> OLSpecId {
        self.cur_spec
    }

    /// Returns the raw spec version the chainstate was produced under.
    pub fn cur_spec_version(&self) -> u32 {
        self.cur_spec.into()
    }

    /// Returns the raw spec version the next epoch runs under.
    pub fn staged_spec_version(&self) -> u32 {
        self.staged_spec_version
    }

    /// Converts [`Self::staged_spec_version`] to the spec it names.
    pub fn staged_spec(&self) -> Result<OLSpecId, UnknownOLSpecId> {
        OLSpecId::try_from(self.staged_spec_version)
    }
}

/// Error returned when a V0 state would stage a spec other than V0.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("V0 OL state cannot stage spec version {staged_spec_version}")]
pub struct NonCanonicalV0Versions {
    staged_spec_version: u32,
}

impl NonCanonicalV0Versions {
    /// Returns the raw staged version the V0 state named.
    pub fn staged_spec_version(&self) -> u32 {
        self.staged_spec_version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v0_versions_must_stage_v0() {
        assert!(OLSpecVersions::new(OLSpecId::V0, 0).is_ok());
        for staged in [1, 2, u32::MAX] {
            let err = OLSpecVersions::new(OLSpecId::V0, staged).unwrap_err();
            assert_eq!(err.staged_spec_version(), staged);
        }
        // From V1 on the staged version is committed, so any value is kept.
        for staged in [0, 1, 2, u32::MAX] {
            let versions = OLSpecVersions::new(OLSpecId::V1, staged).unwrap();
            assert_eq!(versions.staged_spec_version(), staged);
        }
    }
}
