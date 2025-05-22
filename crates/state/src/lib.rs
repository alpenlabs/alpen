#![allow(stable_features)] // FIX: this is needed for sp1 toolchain.
#![feature(is_sorted, is_none_or)]

//! Rollup types relating to the consensus-layer state of the rollup.
//!
//! Types relating to the execution-layer state are kept generic, not
//! reusing any Reth types.

pub mod block;
pub mod block_validation;
pub mod bridge_ops;
pub mod bridge_state;
pub mod chain_state;
pub mod client_state;
pub mod csm_status;
pub mod da;
pub mod exec_env;
pub mod exec_update;
pub mod forced_inclusion;
pub mod genesis;
pub mod header;
pub mod id;
pub mod l1;
pub mod operation;
pub mod state_op;
pub mod state_queue;
pub mod sync_event;
pub mod traits;

pub mod prelude;

pub use strata_primitives::batch;
