//! Helpers for verifying auxiliary data inside ASM subprotocols.
//!
//! Before handing auxiliary responses to `process_txs`, call [`verify_aux_input`]
//! to check the supplied proofs against the header MMR and obtain a verified
//! [`AuxInput`](strata_asm_common::AuxInput).

mod verification;

pub use verification::verify_aux_input;
