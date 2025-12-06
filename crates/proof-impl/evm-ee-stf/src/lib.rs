//! EVM Execution Environment STF for Alpen prover using trait-based ExecutionEnvironment.
//! Provides primitives and utilities to process Ethereum block transactions and state transitions in a zkVM.
pub mod primitives;
pub mod program;

use std::sync::Arc;

use alloy_consensus::Header;
use reth_chainspec::ChainSpec;
use reth_primitives_traits::Block as RethBlock;
use revm_primitives::B256;
use rsp_client_executor::io::EthClientExecutorInput;
use rsp_primitives::genesis::Genesis;
use strata_ee_acct_types::{ExecBlock, ExecBlockOutput, ExecPayload, ExecutionEnvironment};
use strata_ee_chain_types::BlockInputs;
use strata_evm_ee::{EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader, EvmPartialState};
use zkaleido::ZkVmEnv;

pub use primitives::{EvmBlockStfInput, EvmBlockStfOutput};

/// Converts genesis configuration to chain specification.
fn create_chain_spec(genesis: &Genesis) -> Arc<ChainSpec> {
    Arc::new(
        genesis
            .try_into()
            .expect("Failed to convert genesis to chain spec"),
    )
}

/// Converts EthClientExecutorInput EVM block and partial state.
fn convert_block_input(input: EthClientExecutorInput) -> (EvmPartialState, EvmBlock, Header) {
    let state = EvmPartialState::new(input.parent_state, input.bytecodes, input.ancestor_headers);

    let header = input.current_block.header().clone();
    let evm_header = EvmHeader::new(header.clone());
    let block_body = input.current_block.into_body();
    let evm_body = EvmBlockBody::from_alloy_body(block_body);
    let block = EvmBlock::new(evm_header, evm_body);

    (state, block, header)
}

/// Validates block hash continuity in a chain.
fn validate_block_hash_continuity(
    current_blockhash: &mut Option<B256>,
    prev_blockhash: B256,
    new_blockhash: B256,
) {
    if let Some(expected_hash) = current_blockhash {
        assert_eq!(prev_blockhash, *expected_hash, "Block hash mismatch");
    }
    *current_blockhash = Some(new_blockhash);
}

/// Executes a single block using the ExecutionEnvironment trait.
fn execute_single_block(
    env: &EvmExecutionEnvironment,
    state: &EvmPartialState,
    block: &EvmBlock,
    header: &Header,
    block_idx: u32,
) -> ExecBlockOutput<EvmExecutionEnvironment> {
    let exec_payload = ExecPayload::new(header, block.get_body());
    let block_inputs = BlockInputs::new_empty();

    let output = env
        .execute_block_body(state, &exec_payload, &block_inputs)
        .unwrap_or_else(|_| panic!("Failed to execute block {}", block_idx));

    env.verify_outputs_against_header(block.get_header(), &output)
        .unwrap_or_else(|_| panic!("Failed to verify block {} outputs", block_idx));

    output
}

/// Processes a sequence of EVM blocks using the ExecutionEnvironment trait.
///
/// This function reads blocks from the zkVM environment, processes each block
/// using the EvmExecutionEnvironment trait implementation, and commits the outputs.
pub fn process_evm_blocks(zkvm: &impl ZkVmEnv) {
    let num_blocks: u32 = zkvm.read_serde();
    assert!(num_blocks > 0, "At least one block is required.");

    let mut current_blockhash = None;

    for block_idx in 0..num_blocks {
        let input: EthClientExecutorInput = zkvm.read_serde();

        let chain_spec = create_chain_spec(&input.genesis);
        let env = EvmExecutionEnvironment::new(chain_spec);

        let (mut state, block, header) = convert_block_input(input);

        let prev_blockhash = header.parent_hash;
        let new_blockhash = header.hash_slow();
        validate_block_hash_continuity(&mut current_blockhash, prev_blockhash, new_blockhash);

        let output = execute_single_block(&env, &state, &block, &header, block_idx);

        env.merge_write_into_state(&mut state, output.write_batch())
            .unwrap_or_else(|_| panic!("Failed to merge state for block {}", block_idx));

        // TODO: Use outputs of Block execution
        // Need to decide approach:
        // 1. Add transformation layer here ie. BlockOutputs to ExecSegment (backward compatible)
        // 2. Update CL_STF to consume BlockOutputs directly (cleaner long-term)
        // Decision to be made in PR review.
        let _block_outputs = output.outputs();
    }

    // TODO: Commit properly transformed outputs once decision is made
    let empty_outputs: Vec<u8> = vec![];
    zkvm.commit_serde(&empty_outputs);
}
