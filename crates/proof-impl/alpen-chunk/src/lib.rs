//! EE chunk proof implementation wrapping `ee-chunk-runtime` with zkaleido proof IO.

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use rkyv::rancor::Error as RkyvError;
use rsp_primitives::genesis::Genesis;
use strata_ee_chunk_runtime::ArchivedPrivateInput;
use strata_evm_ee::EvmExecutionEnvironment;
use zkaleido::ZkVmEnvSerde;

mod program;

pub use program::{EeChunkProgram, EeChunkProofInput};

/// Guest entry point for EE chunk proof generation.
///
/// Reads a genesis config and an rkyv-serialized private input from the zkVM,
/// verifies the chunk transition using the EVM execution environment, and
/// commits the resulting [`strata_ee_chain_types::ChunkTransition`] as SSZ
/// public output.
pub fn process_ee_chunk(zkvm: &impl ZkVmEnvSerde) {
    let genesis: Genesis = zkvm.read_serde();
    let chain_spec: Arc<ChainSpec> = Arc::new((&genesis).try_into().unwrap());

    let buf = zkvm.read_buf();
    let input: &ArchivedPrivateInput = rkyv::access::<ArchivedPrivateInput, RkyvError>(&buf)
        .expect("failed to access rkyv archive");

    let ee = EvmExecutionEnvironment::new(chain_spec);

    strata_ee_chunk_runtime::verify_input(&ee, input).expect("chunk verification failed");

    zkvm.commit_buf(input.chunk_transition_ssz());
}

/// No-op variant of [`process_ee_chunk`] for native dev/test runs only.
///
/// Reads the same inputs as [`process_ee_chunk`] (so the host's input cursor
/// advances correctly), but skips [`strata_ee_chunk_runtime::verify_input`]
/// — which is the EVM-execution heavy lift. The caller is expected to
/// trust the input bytes already produced by the alpen-client's local block
/// builder. Output bytes are identical to the verifying path so downstream
/// consumers (the acct prover, the OL update verifier under
/// `PredicateKey::always_accept`) are unaffected.
///
/// Wired in `bin/alpen-client/src/main.rs` behind the
/// `--dev-native-noop-prover` flag, which itself only takes effect when
/// `--dev-native-prover` is also set.
pub fn process_ee_chunk_noop(zkvm: &impl ZkVmEnvSerde) {
    let _genesis: Genesis = zkvm.read_serde();
    let buf = zkvm.read_buf();
    let input: &ArchivedPrivateInput = rkyv::access::<ArchivedPrivateInput, RkyvError>(&buf)
        .expect("failed to access rkyv archive");
    zkvm.commit_buf(input.chunk_transition_ssz());
}
