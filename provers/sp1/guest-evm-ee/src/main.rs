// These two lines are necessary for the program to properly compile.
//
// Under the hood, we wrap your main function with some extra code so that it behaves properly
// inside the zkVM.
#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use rsp_client_executor::io::EthClientExecutorInput;
use strata_ee_acct_types::{ExecBlock, ExecPayload, ExecutionEnvironment};
use strata_ee_chain_types::BlockInputs;
use strata_evm_ee::{EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader, EvmPartialState};
use zkaleido::ZkVmEnv;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

fn main() {
    process_evm_blocks(&Sp1ZkVmEnv)
}

/// Processes a sequence of EVM blocks using the ExecutionEnvironment trait.
///
/// This function reads blocks from the zkVM environment, processes each block
/// using the EvmExecutionEnvironment trait implementation, and commits the outputs.
fn process_evm_blocks(zkvm: &Sp1ZkVmEnv) {
    // Read the number of blocks to process
    let num_blocks: u32 = zkvm.read_serde();
    assert!(num_blocks > 0, "At least one block is required.");

    for block_idx in 0..num_blocks {
        // Read the EthClientExecutorInput for this block (same format as existing code)
        let input: EthClientExecutorInput = zkvm.read_serde();

        // Convert to chain spec and create execution environment
        let chain_spec: Arc<ChainSpec> = Arc::new(
            (&input.genesis)
                .try_into()
                .expect("Failed to convert genesis to chain spec"),
        );
        let env = EvmExecutionEnvironment::new(chain_spec);

        // Convert RSP types to strata-evm-ee types
        let mut state = EvmPartialState::new(
            input.parent_state,
            input.bytecodes,
            input.ancestor_headers,
        );

        // Convert the block (taking ownership from input)
        use reth_primitives_traits::Block as RethBlock;
        let header = input.current_block.header().clone();  // Required: Header is needed for both EvmHeader and ExecPayload
        let evm_header = EvmHeader::new(header.clone());

        let block_body = input.current_block.into_body();
        let evm_body = EvmBlockBody::from_alloy_body(block_body);

        let block = EvmBlock::new(evm_header, evm_body);

        // Create execution payload and block inputs
        // Note: Using empty BlockInputs for now - deposits from orchestration layer would go here
        let exec_payload = ExecPayload::new(&header, block.get_body());
        let block_inputs = BlockInputs::new_empty();

        // Execute the block body
        let output = env
            .execute_block_body(&state, &exec_payload, &block_inputs)
            .unwrap_or_else(|_| panic!("Failed to execute block {}", block_idx));

        // Verify outputs against header
        env.verify_outputs_against_header(block.get_header(), &output)
            .unwrap_or_else(|_| panic!("Failed to verify block {} outputs", block_idx));

        // Merge write batch into state
        env.merge_write_into_state(&mut state, output.write_batch())
            .unwrap_or_else(|_| panic!("Failed to merge state for block {}", block_idx));

        // Note: output.outputs() contains BlockOutputs (messages to orchestration layer)
        // For cycle comparison purposes, we don't need to transform this yet
    }

    // Commit empty vector for now (sufficient for cycle comparison)
    // TODO: When integrating into full chain, transform BlockOutputs -> ExecSegment
    let empty_outputs: Vec<u8> = vec![];
    zkvm.commit_serde(&empty_outputs);
}
