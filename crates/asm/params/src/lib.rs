//! Configuration parameters for the Anchor State Machine (ASM).
//!
//! Provides [`AsmParams`], which bundles the L1 magic bytes, genesis L1 view,
//! and per-subprotocol configuration (admin, bridge, checkpoint) needed to
//! initialize and run an ASM instance.

pub mod params;
pub mod subprotocols;
