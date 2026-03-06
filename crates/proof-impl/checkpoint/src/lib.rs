//! Passthrough checkpoint proof that reads [`BatchInfo`] and commits it back unchanged.
//!
//! This proof trait will be fully removed in the future.

use strata_checkpoint_types::BatchInfo;
use zkaleido::ZkVmEnv;

pub mod program;

pub fn process_checkpoint_proof(zkvm: &impl ZkVmEnv) {
    let output: BatchInfo = zkvm.read_borsh();
    zkvm.commit_borsh(&output);
}
