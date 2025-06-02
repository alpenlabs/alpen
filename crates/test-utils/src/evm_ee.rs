use std::path::PathBuf;

use strata_primitives::buf::{Buf32, Buf64};
use strata_proofimpl_evm_ee_stf::{
    primitives::{EvmEeProofInput, EvmEeProofOutput},
    process_block_transaction,
    processor::EvmConfig,
    utils::generate_exec_update,
    EvmBlockStfInput,
};
use strata_state::{
    block::{L1Segment, L2Block, L2BlockBody},
    chain_state::Chainstate,
    header::{L2BlockHeader, L2Header, SignedL2BlockHeader},
    state_op::StateCache,
};

use crate::{
    bitcoin_mainnet_segment::BtcChainSegment,
    l2::{gen_params, get_genesis_chainstate},
};

/// Represents a segment of EVM execution by holding inputs and outputs for
/// block heights within a range. This struct is used to simulate EVM proof
/// generation and processing for testing STF proofs.
#[derive(Debug, Clone)]
pub struct EvmSegment {
    inputs: EvmEeProofInput,
    outputs: EvmEeProofOutput,
}

impl EvmSegment {
    /// Initializes the EvmSegment by loading existing [`EvmBlockStfInput`] data from the specified
    /// range of block heights and generating corresponding ElBlockStfOutput.
    ///
    /// This function reads witness data from JSON files, processes them, and stores the results
    /// for testing purposes of the STF proofs.
    ///
    /// Note: This assumes all the l1 segment is empty
    pub fn initialize_from_saved_ee_data(start_height: u64, end_height: u64) -> Self {
        use revm::primitives::SpecId;

        const EVM_CONFIG: EvmConfig = EvmConfig {
            chain_id: 2892,
            spec_id: SpecId::SHANGHAI,
        };

        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/evm_ee/");
        for height in start_height..=end_height {
            let witness_path = dir.join(format!("witness_{height}.json"));
            let json_file = std::fs::read_to_string(witness_path).expect("Expected JSON file");
            let el_proof_input: EvmBlockStfInput =
                serde_json::from_str(&json_file).expect("Invalid JSON file");
            inputs.push(el_proof_input.clone());

            let block_stf_output = process_block_transaction(el_proof_input, EVM_CONFIG);
            let exec_output = generate_exec_update(&block_stf_output);
            outputs.push(exec_output);
        }

        Self { inputs, outputs }
    }

    /// Retrieves the [`EvmEeProofInput`]
    pub fn get_inputs(&self) -> &EvmEeProofInput {
        &self.inputs
    }

    /// Retrieves the [`EvmEeProofOutput`]
    pub fn get_outputs(&self) -> &EvmEeProofOutput {
        &self.outputs
    }
}

/// Represents a segment of L2 blocks and their associated state transitions.
/// This struct stores L2 blocks, pre-state, and post-state data, simulating
/// the block processing for testing STF proofs.
pub struct L2Segment {
    pub blocks: Vec<L2Block>,
    pub pre_states: Vec<Chainstate>,
    pub post_states: Vec<Chainstate>,
}

impl L2Segment {
    /// Initializes the L2Segment by reconstructing valid L2 blocks and state transitions
    /// from the existing EVM EE segment data. The segment is created using the execution
    /// segment and EVM proof data for the specified block heights.
    ///
    /// This function ensures that valid L2 segments and blocks are generated for testing
    /// the STF proofs by simulating state transitions from a starting genesis state.
    pub fn initialize_from_saved_evm_ee_data(start_block: u64, end_block: u64) -> Self {
        let evm_segment = EvmSegment::initialize_from_saved_ee_data(start_block, end_block);

        let params = gen_params();
        let mut blocks = Vec::new();
        let mut pre_states = Vec::new();
        let mut post_states = Vec::new();

        let (prev_block_bundle, mut prev_chainstate) = get_genesis_chainstate(&params);
        let (mut prev_block, _) = prev_block_bundle.into_parts();

        let el_proof_ins = evm_segment.get_inputs();
        let el_proof_outs = evm_segment.get_outputs();

        for (idx, (el_proof_in, el_proof_out)) in
            el_proof_ins.iter().zip(el_proof_outs.iter()).enumerate()
        {
            // If it is a terminal block, fill L1Segment
            let genesis_height = params.rollup().genesis_l1_height;
            let l1_segment = if idx == evm_segment.get_inputs().len() - 1 {
                let starting_height = genesis_height + 1;
                let len = 3;
                let new_height = starting_height + len - 1; // because inclusive
                let manifests = BtcChainSegment::load()
                    .get_block_manifests(starting_height, len as usize)
                    .expect("fetch manifests");
                L1Segment::new(new_height, manifests)
            } else {
                L1Segment::new_empty(genesis_height)
            };
            let body = L2BlockBody::new(l1_segment, el_proof_out.clone());

            let slot = prev_block.header().slot() + 1;
            let ts = el_proof_in.timestamp;
            let prev_block_id = prev_block.header().get_blockid();

            let fake_header = L2BlockHeader::new(slot, 0, ts, prev_block_id, &body, Buf32::zero());

            let pre_state = prev_chainstate.clone();
            let mut state_cache = StateCache::new(pre_state.clone());
            strata_chaintsn::transition::process_block(
                &mut state_cache,
                &fake_header,
                &body,
                params.rollup(),
            )
            .unwrap();
            let post_state = state_cache.finalize().into_toplevel();
            let new_state_root = post_state.compute_state_root();

            let header = L2BlockHeader::new(slot, 0, ts, prev_block_id, &body, new_state_root);
            let signed_header = SignedL2BlockHeader::new(header, Buf64::zero()); // TODO: fix this
            let block = L2Block::new(signed_header, body);

            // Note: We need to do this double as of now.
            let mut state_cache = StateCache::new(pre_state.clone());
            strata_chaintsn::transition::process_block(
                &mut state_cache,
                block.header(),
                block.body(),
                params.rollup(),
            )
            .unwrap();
            let post_state = state_cache.finalize().into_toplevel();

            blocks.push(block.clone());
            pre_states.push(pre_state);
            post_states.push(post_state.clone());

            prev_block = block;
            prev_chainstate = post_state;
        }

        L2Segment {
            blocks,
            pre_states,
            post_states,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::L2Segment;

    #[test]
    fn test_chaintsn() {
        let start_height = 1;
        let end_height = 4;
        let l2_segment = L2Segment::initialize_from_saved_evm_ee_data(start_height, end_height);

        for height in start_height..end_height - 1 {
            let pre_state = &l2_segment.pre_states[height as usize + 1];
            let post_state = &l2_segment.post_states[height as usize];
            assert_eq!(pre_state, post_state);
        }
    }
}
