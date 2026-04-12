//! Passthrough checkpoint proof that reads [`BatchInfo`] and commits it back unchanged.
//!
//! This proof trait will be fully removed in the future.

use strata_checkpoint_types::BatchInfo;
use zkaleido::ZkVmEnvBorsh;

pub mod program;

pub fn process_checkpoint_proof(zkvm: &impl ZkVmEnvBorsh) {
    let output: BatchInfo = zkvm.read_borsh();
    zkvm.commit_borsh(&output);
}
