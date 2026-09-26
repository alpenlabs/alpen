//! Versioned container for the OL state.
//!
//! [`OLStateContainer`] pairs the fixed [`OLRootState`] with the chainstate,
//! held as an [`OLStateSeries`] variant in the layout that the root's current
//! spec maps to. The container is what storage, the chain worker, and the
//! checkpoint proof input carry, so that a restored state reproduces the
//! committed root.
//!
//! # Commitments
//!
//! - The protocol state root is the SSZ hash tree root of [`OLRootState`] over exactly its three
//!   fields. Block headers, checkpoint terminal headers, and genesis commit to it. Neither the
//!   container nor [`OLStateSeries`] is ever hashed as such.
//! - `chainstate_root` is the hash tree root of the layout's own state type
//!   ([`OLStateV1`](strata_ol_state_types_v1::OLStateV1) for [`OLStateLayout::V1`]), with no union
//!   selector mixed in.
//!
//! # Layout selection
//!
//! [`OLStateLayout::for_spec`] maps the spec the chainstate was produced under,
//! `cur_spec_version`, to its layout. The staged spec never selects the
//! layout, and spec and layout numbers are never compared: several specs can
//! share one layout.
//!
//! # Serialization
//!
//! Only the root is a consensus structure, so the container has no wire
//! format of its own. It implements serde through a form that holds the
//! root's three fields and the chainstate's SSZ encoding. The OL state
//! database stores that form as CBOR, and the checkpoint proof input carries
//! it through the zkVM's serde input.
//!
//! Deserializing maps `cur_spec_version` to a layout, decodes the chainstate
//! in that layout, requires the chainstate bytes to have the canonical
//! encoding's length, and checks that their hash tree root equals
//! `chainstate_root`. Unsupported specs, malformed chainstates, and root
//! mismatches fail with distinct errors, which reach the caller as the
//! deserializer's error.

mod container;
mod errors;
mod layout;
mod series;

pub use container::OLStateContainer;
pub use layout::OLStateLayout;
pub use series::OLStateSeries;
pub use strata_ol_state_types::OLRootState;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

#[cfg(test)]
mod tests;
