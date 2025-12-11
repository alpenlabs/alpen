//! Checkpoint subprotocol test utilities.
//!
//! Provides builders for creating checkpoint transactions, fixture generators
//! for test data, and mock contexts for unit testing.

mod builder;
mod fixtures;

pub use builder::{
    CheckpointTxBuildError, CheckpointTxBuildResult, TEST_MAGIC_BYTES,
    build_checkpoint_envelope_script, create_checkpoint_reveal_tx, create_test_checkpoint_tx,
};
pub use fixtures::{
    CheckpointFixtures, SequencerKeypair, gen_batch_info, gen_batch_transition,
    gen_checkpoint_payload, gen_checkpoint_sidecar, gen_dummy_proof, gen_l1_block_commitment,
    gen_l1_block_range, gen_l2_block_commitment, gen_l2_block_range, gen_random_buf32,
    sign_checkpoint_payload,
};
