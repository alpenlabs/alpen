#![expect(stable_features, reason = "Required for sp1 toolchain compatibility")] // FIX: this is needed for sp1 toolchain.
#![feature(is_sorted, is_none_or)]

//! Rollup types relating to the consensus-layer state of the rollup.
//!
//! Types relating to the execution-layer state are kept generic, not
//! reusing any Reth types.

pub mod asm_state;
pub mod exec_env;
pub mod exec_update;
pub mod forced_inclusion;
pub mod prelude;
pub mod state_queue;
