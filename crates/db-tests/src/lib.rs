//! Generic database unit tests for database trait impls.
//!
//! Each module exports a macro which instantiates all of the unit tests for
//! the database kind so that when new unit tests are added, all impls
//! automatically inherit them.

pub mod asm_tests;
pub mod chain_state_tests;
pub mod checkpoint_tests;
pub mod client_state_tests;
pub mod l1_broadcast_tests;
pub mod l1_tests;
pub mod l1_writer_tests;
pub mod l2_tests;
pub mod proof_tests;
