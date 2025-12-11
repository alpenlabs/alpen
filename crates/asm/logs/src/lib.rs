//! # ASM Log Types
//!
//! This crate provides structured log types for the Anchor State Machine (ASM) in the Strata
//! protocol. It defines various log entry types that capture important events within the system.
//!
//! # Checkpoint Update Types
//!
//! This crate provides two checkpoint update log types:
//!
//! - [`CheckpointUpdate`]: Uses new SPS-62 checkpoint types. Used by the checkpoint subprotocol.
//! - [`CheckpointUpdateLegacy`]: Uses old checkpoint types. Used by csm-worker for backward
//!   compatibility.
//!
//! TODO(cleanup): Remove `CheckpointUpdateLegacy` when csm-worker is deprecated after OL STF
//! migration.

pub mod asm_stf;
pub mod checkpoint;
pub mod checkpoint_legacy;
pub mod constants;
pub mod deposit;
pub mod export;
pub mod forced_inclusion;

pub use asm_stf::AsmStfUpdate;
pub use checkpoint::CheckpointUpdate;
// Legacy checkpoint update for csm-worker compatibility
// TODO(cleanup): Remove when csm-worker is deprecated
pub use checkpoint_legacy::CheckpointUpdateLegacy;
pub use deposit::DepositLog;
pub use export::NewExportEntry;
pub use forced_inclusion::ForcedInclusionData;
