//! Input types for Alpen EE proof generation

mod account_init;
mod block_package;
mod proof_input;
mod runtime_input;

pub use account_init::EeAccountInit;
pub use block_package::CommitBlockPackage;
pub use proof_input::AlpenEeProofInput;
pub use runtime_input::RuntimeUpdateInput;
