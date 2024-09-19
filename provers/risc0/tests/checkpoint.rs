#[cfg(feature = "prover")]
mod test {
    use std::str::FromStr;

    use alpen_test_utils::l2::get_genesis_chainstate;
    use bitcoin::{params::MAINNET, Address};
    use express_proofimpl_btc_blockspace::logic::{BlockspaceProofOutput, ScanRuleConfig};
    use express_proofimpl_checkpoint::{
        CheckpointProofOutput, HashedCheckpointState, L2BatchProofOutput,
    };
    use express_proofimpl_l1_batch::{
        logic::{L1BatchProofInput, L1BatchProofOutput},
        mock::get_verification_state_for_block,
        pow_params::PowParams,
    };
    use express_risc0_adapter::{Risc0Verifier, RiscZeroHost};
    use express_risc0_guest_builder::{
        GUEST_RISC0_BTC_BLOCKSPACE_ELF, GUEST_RISC0_BTC_BLOCKSPACE_ID, GUEST_RISC0_CHECKPOINT_ELF,
        GUEST_RISC0_L1_BATCH_ELF, GUEST_RISC0_L1_BATCH_ID,
    };
    use express_zkvm::{
        AggregationInput, Proof, ProverInput, ProverOptions, VerificationKey, ZKVMHost,
        ZKVMVerifier,
    };

    // TODO: handle this repeat
    fn get_l1_batch_output_and_proof() -> (L1BatchProofOutput, Proof) {
        let mainnet_blocks: Vec<(u32, String)> = vec![
            (40321, "0100000045720d24eae33ade0d10397a2e02989edef834701b965a9b161e864500000000993239a44a83d5c427fd3d7902789ea1a4d66a37d5848c7477a7cf47c2b071cd7690784b5746651c3af7ca030101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff08045746651c02db00ffffffff0100f2052a01000000434104c9f513361104db6a84fb6d5b364ba57a27cd19bd051239bf750d8999c6b437220df8fea6b932a248df3cad1fdebb501791e02b7b893a44718d696542ba92a0acac00000000".to_owned()),
        ];

        let prover_options = ProverOptions {
            use_mock_prover: false,
            stark_to_snark_conversion: false,
            enable_compression: false,
        };
        let prover = RiscZeroHost::init(
            GUEST_RISC0_BTC_BLOCKSPACE_ELF.into(),
            // Default::default(),
            prover_options,
        );

        let btc_blockspace_elf_id: Vec<u8> = GUEST_RISC0_BTC_BLOCKSPACE_ID
            .iter()
            .flat_map(|&x| x.to_le_bytes())
            .collect();

        let mut blockspace_outputs = Vec::new();
        let mut prover_input = ProverInput::new();
        for (_, raw_block) in mainnet_blocks {
            let block_bytes = hex::decode(&raw_block).unwrap();
            let scan_config = ScanRuleConfig {
                bridge_scriptbufs: vec![Address::from_str(
                    "bcrt1pf73jc96ujch43wp3k294003xx4llukyzvp0revwwnww62esvk7hqvarg98",
                )
                .unwrap()
                .assume_checked()
                .script_pubkey()],
            };
            let mut inner_prover_input = ProverInput::new();
            inner_prover_input.write(scan_config.clone());
            inner_prover_input.write_serialized(block_bytes);

            let (proof, _) = prover
                .prove(&inner_prover_input)
                .expect("Failed to generate proof");

            let output = Risc0Verifier::extract_public_output::<BlockspaceProofOutput>(&proof)
                .expect("Failed to extract public outputs");

            prover_input.write_proof(AggregationInput::new(
                proof,
                VerificationKey::new(btc_blockspace_elf_id.clone()),
            ));
            blockspace_outputs.push(output);
        }

        let prover = RiscZeroHost::init(GUEST_RISC0_L1_BATCH_ELF.into(), prover_options);
        let input = L1BatchProofInput {
            batch: blockspace_outputs,
            state: get_verification_state_for_block(40321, &PowParams::from(&MAINNET)),
        };

        prover_input.write(input);
        let (proof, _) = prover
            .prove(&prover_input)
            .expect("Failed to generate proof");

        let output = Risc0Verifier::extract_public_output::<L1BatchProofOutput>(&proof)
            .expect("Failed to extract public outputs");

        (output, proof)
    }

    // fn get_l1_batch_output() -> L1BatchProofOutput {
    //     let params = PowParams::from(&MAINNET);
    //     L1BatchProofOutput {
    //         deposits: Vec::new(),
    //         forced_inclusions: Vec::new(),
    //         state_update: None,
    //         initial_state: get_verification_state_for_block(40_320, &params),
    //         final_state: get_verification_state_for_block(40_321, &params),
    //     }
    // }

    fn get_l2_batch_output() -> L2BatchProofOutput {
        L2BatchProofOutput {
            deposits: Vec::new(),
            forced_inclusions: Vec::new(),
            initial_state: get_genesis_chainstate(),
            final_state: get_genesis_chainstate(),
        }
    }

    #[test]
    fn test_checkpoint_proof() {
        let (l1_batch, l1_batch_proof) = get_l1_batch_output_and_proof();
        // let l1_batch = get_l1_batch_output();
        let l2_batch = get_l2_batch_output();

        let genesis = HashedCheckpointState {
            l1_state: l1_batch.initial_state.hash().unwrap(),
            l2_state: l2_batch.initial_state.compute_state_root(),
        };

        let prover_options = ProverOptions {
            use_mock_prover: false,
            stark_to_snark_conversion: false,
            enable_compression: false,
        };
        let prover = RiscZeroHost::init(GUEST_RISC0_CHECKPOINT_ELF.into(), prover_options);

        let l1_batch_image_id: Vec<u8> = GUEST_RISC0_L1_BATCH_ID
            .iter()
            .flat_map(|&x| x.to_le_bytes())
            .collect();
        let l1_batch_proof_input = AggregationInput::new(
            l1_batch_proof,
            VerificationKey::new(l1_batch_image_id.clone()),
        );

        let mut prover_input = ProverInput::new();
        prover_input.write(l1_batch);
        prover_input.write_serialized(borsh::to_vec(&l2_batch).unwrap());
        prover_input.write_serialized(borsh::to_vec(&genesis).unwrap());

        prover_input.write_proof(l1_batch_proof_input);

        let (proof, _) = prover
            .prove(&prover_input)
            .expect("Failed to generate proof");

        let _output = Risc0Verifier::extract_public_output::<CheckpointProofOutput>(&proof)
            .expect("Failed to extract public outputs");
    }
}
