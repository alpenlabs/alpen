//! Test utilities for L2 (Orchestration Layer) components.

// TODO: (@PG) remove the legacy code
mod legacy;
pub use legacy::{
    gen_l2_chain, gen_params, get_genesis_chainstate, get_test_operator_secret_key,
    get_test_signed_checkpoint,
};

mod checkpoint;
pub use checkpoint::CheckpointTestHarness;
