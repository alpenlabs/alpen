//! All the handlers, modifiers and verifiers related to snark accounts.

mod handlers;
mod update;
mod verification;

pub use handlers::{handle_snark_msg, handle_snark_transfer};
pub use update::apply_update_outputs;
pub use verification::{
    verify_message_index, verify_msgs_and_msgs_in_proofs, verify_seq_no, verify_update_correctness,
};
