//! EE chunk proof implementation wrapping `ee-chunk-runtime` with zkaleido proof IO.

use std::sync::Arc;

use alpen_reth_evm::evm::AlpenEvmFactory;
use reth_chainspec::ChainSpec;
use rkyv::rancor::Error as RkyvError;
use rsp_primitives::genesis::Genesis;
use ssz::Decode;
use strata_bridge_params::BridgeParams;
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

    let withdrawal_ssz = zkvm.read_buf();
    let bridge_params = BridgeParams::from_ssz_bytes(&withdrawal_ssz)
        .expect("failed to deserialize withdrawal params");
    let evm_factory = AlpenEvmFactory::from_bridge_params(&bridge_params);
    let ee = EvmExecutionEnvironment::new(chain_spec, evm_factory);

    strata_ee_chunk_runtime::verify_input(&ee, input).expect("chunk verification failed");

    zkvm.commit_buf(input.chunk_transition_ssz());
}
