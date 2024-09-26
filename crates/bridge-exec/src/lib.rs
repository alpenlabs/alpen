//! Executor for bridge duties.
//!
//! Provides methods that allow spawning of async threads that handle the deposit and withdrawal
//! operations as well as the ability to query their status.

pub mod book_keeping;
pub mod config;
pub mod errors;
pub mod handler;
