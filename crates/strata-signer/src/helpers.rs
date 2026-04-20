//! Shared types for the sequencer signer service.

use std::sync::Arc;

use strata_crypto::keys::zeroizable::ZeroizedBuf32;

/// Reference-counted, zeroize-on-drop handle to the sequencer secret key.
///
/// Using [`Arc`] ensures that spawned duty handlers receive a pointer clone
/// rather than a byte-level copy of key material.
pub type SequencerSk = Arc<ZeroizedBuf32>;
