//! Passthrough checkpoint proof that reads [`BatchInfo`] and commits it back unchanged.
//!
//! This proof trait will be fully removed in the future.

use ssz::{Decode, Encode};
use strata_checkpoint_types::BatchInfo;
use zkaleido::ZkVmEnv;

pub mod program;

pub fn process_checkpoint_proof(zkvm: &impl ZkVmEnv) {
    let output = BatchInfo::from_ssz_bytes(&zkvm.read_buf()).expect("ssz batch info input");
    zkvm.commit_buf(&output.as_ssz_bytes());
}
