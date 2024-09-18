#[cfg(feature = "prover")]
mod test {
    use std::str::FromStr;

    use bitcoin::{consensus::serialize, Address};
    use express_proofimpl_btc_blockspace::logic::{BlockspaceProofOutput, ScanRuleConfig};
    use express_risc0_adapter::{Risc0Verifier, RiscZeroHost};
    use express_risc0_guest_builder::ALPEN_BTC_BLOCKSPACE_RISC0_PROOF_ELF;
    use express_zkvm::{ProverInput, ZKVMHost, ZKVMVerifier};

    #[test]
    fn test_btc_blockspace_code_trace_generation() {
        let block = alpen_test_utils::bitcoin::get_btc_mainnet_block();
        let prover = RiscZeroHost::init(
            ALPEN_BTC_BLOCKSPACE_RISC0_PROOF_ELF.into(),
            Default::default(),
        );

        let scan_config = ScanRuleConfig {
            bridge_scriptbufs: vec![Address::from_str(
                "bcrt1pf73jc96ujch43wp3k294003xx4llukyzvp0revwwnww62esvk7hqvarg98",
            )
            .unwrap()
            .assume_checked()
            .script_pubkey()],
        };
        let serialized_block = serialize(&block);

        let mut prover_input = ProverInput::new();
        prover_input.write(scan_config.clone());
        prover_input.write_serialized(serialized_block);

        let (proof, _) = prover
            .prove(&prover_input)
            .expect("Failed to generate proof");

        let _output = Risc0Verifier::extract_public_output::<BlockspaceProofOutput>(&proof)
            .expect("Failed to extract public outputs");
    }
}
