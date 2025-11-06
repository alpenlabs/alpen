//! SNARK account system implementation.
//!
//! This crate provides the core functionality for SNARK-based accounts, including:
//! - Account state updates and transitions
//! - Message and transfer handlers
//! - Verification logic for SNARK updates

mod handlers;
mod update;
mod verification;

pub use handlers::*;
pub use update::*;
pub use verification::*;
