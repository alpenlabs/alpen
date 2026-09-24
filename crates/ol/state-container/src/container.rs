//! The OL state container and its serde form.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ssz::DecodeError;
use strata_identifiers::Buf32;
use strata_ol_state_types::{OLRootState, OLSpecId};

use crate::errors::OLStateDecodeError;
use crate::layout::OLStateLayout;
use crate::series::OLStateSeries;

/// Full OL state: the fixed root and the chainstate it commits to.
///
/// The container is immutable. Every constructor computes or checks
/// `chainstate_root` against the chainstate, so the root never goes stale.
/// Execution mutates a per-layout state accessor built from the container and
/// converts back into a new container afterwards.
///
/// The current spec is always one this binary supports, because it selects
/// the chainstate layout. The staged spec may be unknown: the state still
/// materializes, and executing the next epoch must halt until the node is
/// upgraded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OLStateContainer {
    root: OLRootState,
    chainstate: OLStateSeries,
}

impl OLStateContainer {
    /// Creates a container, computing the chainstate root.
    ///
    /// # Panics
    ///
    /// In debug builds, if `chainstate` is not in the layout `cur_spec` maps
    /// to.
    pub fn new(cur_spec: OLSpecId, staged_spec_version: u32, chainstate: OLStateSeries) -> Self {
        debug_assert_eq!(
            chainstate.layout(),
            OLStateLayout::for_spec(cur_spec),
            "ol/state-container: chainstate layout does not match the current spec"
        );
        let root = OLRootState::new(
            cur_spec.into(),
            staged_spec_version,
            chainstate.compute_chainstate_root(),
        );
        Self { root, chainstate }
    }

    /// Returns the fixed root.
    pub fn root(&self) -> &OLRootState {
        &self.root
    }

    /// Returns the raw spec version the chainstate was produced under.
    pub fn cur_spec_version(&self) -> u32 {
        self.root.cur_spec_version()
    }

    /// Returns the raw spec version the next epoch runs under.
    pub fn staged_spec_version(&self) -> u32 {
        self.root.staged_spec_version()
    }

    /// Returns the spec the chainstate was produced under.
    pub fn cur_spec(&self) -> OLSpecId {
        self.root
            .cur_spec()
            .expect("ol/state-container: current spec checked at construction")
    }

    /// Returns the chainstate.
    pub fn chainstate(&self) -> &OLStateSeries {
        &self.chainstate
    }

    /// Splits the container into its root and chainstate.
    pub fn into_parts(self) -> (OLRootState, OLStateSeries) {
        (self.root, self.chainstate)
    }

    /// Computes the protocol state root, `hash_tree_root(OLRootState)`.
    pub fn compute_state_root(&self) -> Buf32 {
        self.root.compute_state_root()
    }

    pub(crate) fn to_serde(&self) -> SerdeOLStateContainer {
        SerdeOLStateContainer {
            cur_spec_version: self.root.cur_spec_version(),
            staged_spec_version: self.root.staged_spec_version(),
            chainstate_root: self.root.chainstate_root(),
            chainstate: self.chainstate.to_ssz_bytes(),
        }
    }

    /// Selects the chainstate layout from the current spec, decodes the
    /// chainstate in it, and checks the chainstate against its committed root.
    pub(crate) fn from_serde(
        serde_form: SerdeOLStateContainer,
    ) -> Result<Self, OLStateDecodeError> {
        let SerdeOLStateContainer {
            cur_spec_version,
            staged_spec_version,
            chainstate_root,
            chainstate: chainstate_bytes,
        } = serde_form;
        let root = OLRootState::new(cur_spec_version, staged_spec_version, chainstate_root);
        let cur_spec = root
            .cur_spec()
            .map_err(OLStateDecodeError::UnsupportedSpec)?;
        let layout = OLStateLayout::for_spec(cur_spec);

        let chainstate = OLStateSeries::from_ssz_bytes(layout, &chainstate_bytes)
            .map_err(|source| OLStateDecodeError::MalformedChainstate { layout, source })?;

        // The SSZ decoder ignores some bytes, such as a body after the empty
        // variant of a union. Requiring the canonical length rejects such
        // encodings instead of silently accepting a longer encoding of the
        // same state.
        let canonical_len = chainstate.ssz_bytes_len();
        if chainstate_bytes.len() != canonical_len {
            return Err(OLStateDecodeError::MalformedChainstate {
                layout,
                source: DecodeError::InvalidByteLength {
                    len: chainstate_bytes.len(),
                    expected: canonical_len,
                },
            });
        }

        let computed = chainstate.compute_chainstate_root();
        if computed != chainstate_root {
            return Err(OLStateDecodeError::ChainstateRootMismatch {
                committed: chainstate_root,
                computed,
            });
        }

        Ok(Self { root, chainstate })
    }
}

impl Serialize for OLStateContainer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_serde().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OLStateContainer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let serde_form = SerdeOLStateContainer::deserialize(deserializer)?;
        Self::from_serde(serde_form).map_err(D::Error::custom)
    }
}

/// Serde form of [`OLStateContainer`]: the root's fields and the chainstate's
/// SSZ encoding in the layout the current spec maps to.
///
/// The chainstate stays opaque here. Only [`OLStateContainer::from_serde`]
/// interprets it, after the root has selected its layout.
#[derive(Serialize, Deserialize)]
pub(crate) struct SerdeOLStateContainer {
    pub(crate) cur_spec_version: u32,
    pub(crate) staged_spec_version: u32,
    pub(crate) chainstate_root: Buf32,
    #[serde(with = "serde_bytes")]
    pub(crate) chainstate: Vec<u8>,
}
