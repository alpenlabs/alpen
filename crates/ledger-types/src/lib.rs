//! Ledger data types.
//!
//! This crate is NOT about the basic data structures themselves.  This crate
//! focuses on how we access the ledger data structures in different contexts.
//!
//! We present a trait that represents the various types of structures we
//! interact with in the ledger's state, and expose accessor functions on it.
//! The different impls of these traits are tailored for different contexts.  In
//! some contexts we care about tracing DA generation, in others we may be doing
//! blocking fetches from disk we want to trace for later proof generation.
//!
//! We use the `I` prefix convention which is normally uncommon in Rust to refer
//! to these abstract data structures.  This is because the "ordinary" struct
//! versions of these data structure we use on the wire are the "real" versions
//! we want to think of them as being, but these traits are standins for those.
//! Making up new names for these items would crate too much confusion.

mod account;
mod state_accessor;
mod toplevel;

pub use account::{AccountTypeState, IAccountState, ISnarkAccountState};
pub use state_accessor::StateAccessor;
pub use toplevel::IToplevelState;
