//! Helpers for requesting and consuming auxiliary data inside ASM subprotocols.
//!
//! The README in this crate contains a worked example that shows how to register
//! an auxiliary request during `pre_process_txs` and consume the corresponding
//! `AuxResponseEnvelope` via an [`AuxResolver`] in `process_txs`.
//!
//! [`AuxResolver`]: strata_asm_common::AuxResolver

mod collector;
mod request;
mod resolver;
mod response;

pub use collector::AuxRequestCollector;
pub use request::AuxRequestSpec;
pub use resolver::SubprotocolAuxResolver;
pub use response::{AuxResponseEnvelope, HistoricalLogSegment, LogMmrProof};
