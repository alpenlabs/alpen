//! Chain abstraction types.
//!
//! These traits are intended to abstract over the specific block encoding types
//! so that we can handle protocol format changes a little more gracefully.

mod block;
mod object;
mod snark_account_update;
mod transaction;
