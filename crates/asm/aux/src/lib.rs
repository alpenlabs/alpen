//! Helpers for requesting and consuming auxiliary data inside ASM subprotocols.
//!
//! During `pre_process_txs`, subprotocols can use [`RequestCollector`] to record the auxiliary
//! inputs they need for each L1 transaction. Before handing the responses to `process_txs`, call
//! [`verify_aux_input`] to check the supplied proofs against the header MMR and obtain a verified
//! [`AuxInput`](strata_asm_common::AuxInput).

mod collector;
mod request;
mod utils;

pub use collector::RequestCollector;
pub use request::{AuxRequestEnvelope, AuxRequestTable};
pub use utils::verify_aux_input;
