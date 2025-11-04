//! Helpers for verifying auxiliary data inside ASM subprotocols.
//!
//! Before handing auxiliary responses to `process_txs`, call [`verify_aux_input`]
//! to check the supplied proofs against the header MMR and obtain a verified
//! [`VerifiedAuxInput`](strata_asm_common::VerifiedAuxInput).

mod verification;

pub use verification::{AuxVerificationError, verify_aux_input};
