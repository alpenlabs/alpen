use std::sync::Arc;

use alloy_genesis::Genesis;
use reth_chainspec::ChainSpec;
use rkyv::rancor::Error as RkyvError;
use strata_ee_chunk_runtime::ArchivedPrivateInput;
use strata_evm_ee::EvmExecutionEnvironment;
use zkaleido::ZkVmEnv;

pub mod program;

/// Guest entry point for EE chunk proof generation.
///
/// Reads a genesis config and an rkyv-serialized private input from the zkVM,
/// verifies the chunk transition using the EVM execution environment, and
/// commits the resulting [`strata_ee_chain_types::ChunkTransition`] as SSZ
/// public output.
pub fn process_ee_chunk(zkvm: &impl ZkVmEnv) {
    let genesis: Genesis = zkvm.read_serde();
    let chain_spec: Arc<ChainSpec> = Arc::new(genesis.into());

    let buf = zkvm.read_buf();
    let input: &ArchivedPrivateInput = rkyv::access::<ArchivedPrivateInput, RkyvError>(&buf)
        .expect("failed to access rkyv archive");

    let ee = EvmExecutionEnvironment::new(chain_spec);

    strata_ee_chunk_runtime::verify_input(&ee, input).expect("chunk verification failed");

    zkvm.commit_buf(input.chunk_transition_ssz());
}
