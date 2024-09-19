#[cfg(feature = "prover")]
mod test {
    use alpen_express_state::{block::L2Block, chain_state::ChainState};
    use express_risc0_adapter::{Risc0Verifier, RiscZeroHost};
    use express_risc0_guest_builder::GUEST_RISC0_CL_STF_ELF;
    use express_zkvm::{ProverInput, ZKVMHost, ZKVMVerifier};

    fn get_prover_input() -> (ChainState, L2Block) {
        let prev_state_data: &[u8] =
            include_bytes!("../../test-util/cl_stfs/slot-1/prev_chstate.borsh");
        let new_block_data: &[u8] =
            include_bytes!("../../test-util/cl_stfs/slot-1/final_block.borsh");

        let prev_state: ChainState = borsh::from_slice(prev_state_data).unwrap();
        let block: L2Block = borsh::from_slice(new_block_data).unwrap();

        (prev_state, block)
    }

    #[test]
    fn test_reth_stf_guest_code_trace_generation() {
        let input = get_prover_input();
        let input_ser = borsh::to_vec(&input).unwrap();

        let prover = RiscZeroHost::init(GUEST_RISC0_CL_STF_ELF.into(), Default::default());

        // TODO: handle this properly
        let mut prover_input = ProverInput::new();
        prover_input.write(input_ser);
        let (proof, _) = prover
            .prove(&prover_input)
            .expect("Failed to generate proof");

        let new_state_ser = Risc0Verifier::extract_public_output::<Vec<u8>>(&proof)
            .expect("Failed to extract public outputs");

        let _new_state: ChainState = borsh::from_slice(&new_state_ser).unwrap();
    }
}
