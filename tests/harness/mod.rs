//! Common utilities for all integration test "crates".
//!
//! Integration tests in Rust are a bit [weird](https://doc.rust-lang.org/rust-by-example/testing/integration_testing.html).
//!
//! Each file directly under the `tests` directory is treated as a separate crate.
//! So, files like `bridge-in-flow.rs` and `cooperative-withdrawal-flow.rs` are all separate crates
//! for `cargo`. But any file in a subdirectory inside `tests` is considered a module, for example
//! the `bridge.rs` file is a module. It is for this reason, that you will also find a module
//! declaration (`mod common`) in each file directly under the `tests` directory.
//!
//! From the perspective of a test "crate", a function it does not use
//! is `dead code` even though it is actually being used by another test "crate".
//! Apparently, `clippy` isn't smart enough for that kind of analysis.
//!
//! # Module Structure
//!
//! - `harness`: Core test harness (Bitcoin regtest + ASM worker) - subprotocol-agnostic
//! - `worker_context`: ASM worker context implementation for tests
//! - `admin`: Admin subprotocol utilities (AdminExt trait, action builders)
//! - `checkpoint`: Checkpoint subprotocol utilities (CheckpointExt trait)

pub mod admin;
pub mod checkpoint;
pub mod test_harness;
pub mod worker_context;
