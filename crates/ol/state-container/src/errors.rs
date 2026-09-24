//! Errors from decoding an OL state container.

use ssz::DecodeError;
use strata_identifiers::Buf32;
use strata_ol_state_types::UnknownOLSpecId;
use thiserror::Error;

use crate::layout::OLStateLayout;

/// Errors from decoding an [`OLStateContainer`](crate::OLStateContainer).
///
/// Deserialization reports these through the deserializer's error, so each
/// message carries everything a caller sees.
#[derive(Debug, Error)]
pub(crate) enum OLStateDecodeError {
    /// The root's current spec names no spec this binary supports, so its
    /// chainstate layout is unknown.
    #[error("OL state was produced under unsupported spec version {}", .0.raw())]
    UnsupportedSpec(#[source] UnknownOLSpecId),

    /// The chainstate bytes are not the canonical SSZ encoding of a chainstate
    /// in the layout the root selects.
    #[error("malformed {layout} OL chainstate: {source}")]
    MalformedChainstate {
        layout: OLStateLayout,
        #[source]
        source: DecodeError,
    },

    /// The decoded chainstate does not hash to the root's `chainstate_root`.
    #[error("OL chainstate root mismatch (committed {committed}, computed {computed})")]
    ChainstateRootMismatch { committed: Buf32, computed: Buf32 },
}
