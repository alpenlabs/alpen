//! All the handlers, modifiers and verifiers related to snark accounts.

mod handlers;
mod update;
mod verification;

pub use handlers::{handle_snark_msg, handle_snark_transfer};
pub use update::apply_update_outputs;
pub use verification::{VerifiedUpdate, verify_update_correctness};
