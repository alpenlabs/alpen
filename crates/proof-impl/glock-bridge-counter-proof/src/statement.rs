use bitcoin::Work;
use moho_types::{MohoAttestation, MohoState, StateRefAttestation};
use strata_asm_common::AnchorState;
use strata_asm_moho_program_impl::{AsmStfProgram, MohoProgram};
use strata_asm_proto_bridge_v1::OperatorClaimUnlock;
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::proof::RollupVerifyingKey;
use zkaleido::ProofReceipt;

#[derive(Debug, Clone, Copy)]
pub struct BridgeCounterProofPublicOutput {
    pub tip_total_work: Work,
    pub deposit_idx: u32,
    pub operator_idx: u32,
}

const BRIDGE_ID: u16 = 0;
const MOHO_VK: RollupVerifyingKey = RollupVerifyingKey::NativeVerifyingKey;

#[derive(Debug)]
pub struct BridgeCounterProofInput {
    pub moho_recursive_proof: ProofReceipt,
    pub moho_state: MohoState,
    pub anchor_state: AnchorState,
    pub claim_entry_id: u32,
}

pub fn process_bridge_counter_proof(
    input: &BridgeCounterProofInput,
    genesis_moho: &StateRefAttestation,
) -> BridgeCounterProofPublicOutput {
    let proof = &input.moho_recursive_proof;

    // Verify the recursive MOHO proof and reconstruct its public parameters
    verify_rollup_groth16_proof_receipt(proof, &MOHO_VK).expect("Failed to verify groth16 proof");

    let moho_attestation = borsh::from_slice::<MohoAttestation>(proof.public_values().as_bytes())
        .expect("Failed to decode public params");

    // Ensure the proof chain starts from the canonical genesis state
    assert_eq!(moho_attestation.genesis(), genesis_moho, "Genesis mismatch");

    // Verify that the provided MOHO state matches the committed attestation
    assert_eq!(
        &input.moho_state.compute_commitment(),
        moho_attestation.proven().commitment(),
        "State commitment mismatch"
    );

    // Locate and extract the operator claim from the bridge sub-protocol export logs
    let export_state = input.moho_state.export_state();

    let bridge_container = export_state
        .containers()
        .iter()
        .find(|x| x.container_id() == BRIDGE_ID)
        .expect("Bridge container not found in moho state");

    let export_entry = bridge_container
        .entries()
        .iter()
        .find(|x| x.entry_id() == input.claim_entry_id)
        .expect("Bridge entry not found in container");

    let operator_claim = borsh::from_slice::<OperatorClaimUnlock>(export_entry.payload())
        .expect("Failed to deserialize OperatorClaimUnlock");

    // Verify the anchor state commitment matches Moho's inner state
    assert_eq!(
        input.moho_state.inner_state(),
        AsmStfProgram::compute_state_commitment(&input.anchor_state),
        "Inner state commitment mismatch"
    );

    // Extract the total accumulated proof-of-work
    let tip_total_work = input
        .anchor_state
        .chain_view
        .pow_state
        .total_accumulated_pow
        .clone()
        .into();

    BridgeCounterProofPublicOutput {
        deposit_idx: operator_claim.deposit_idx,
        operator_idx: operator_claim.operator_idx,
        tip_total_work,
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{Txid, hashes::Hash};
    use moho_types::{
        ExportContainer, ExportEntry, ExportState, MohoStateCommitment, StateReference,
    };
    use strata_asm_common::ChainViewState;
    use strata_asm_types::{BtcWork, HeaderVerificationState};
    use strata_primitives::l1::BitcoinTxid;
    use zkaleido::{Proof, PublicValues, VerifyingKey};

    use super::*;

    fn gen_mock_operator_claim_unlock(deposit_idx: u32, operator_idx: u32) -> OperatorClaimUnlock {
        OperatorClaimUnlock {
            withdrawal_txid: BitcoinTxid::new(&Txid::all_zeros()),
            deposit_txid: BitcoinTxid::new(&Txid::all_zeros()),
            deposit_idx,
            operator_idx,
        }
    }

    fn gen_mock_genesis_commitment() -> StateRefAttestation {
        StateRefAttestation::new(
            StateReference::new([0u8; 32]),
            MohoStateCommitment::new([0u8; 32]),
        )
    }

    fn gen_mock_bridge_counter_proof_input(
        genesis_ref: &StateRefAttestation,
        operator_claim_payload: Vec<u8>,
        claim_entry_id: u32,
        header_verification_state: HeaderVerificationState,
    ) -> BridgeCounterProofInput {
        let anchor_state = AnchorState {
            chain_view: ChainViewState {
                pow_state: header_verification_state,
            },
            sections: vec![],
        };

        let inner_state_commitment = AsmStfProgram::compute_state_commitment(&anchor_state);

        let export_entry = ExportEntry::new(claim_entry_id, operator_claim_payload);
        let bridge_container = ExportContainer::new(BRIDGE_ID, vec![], vec![export_entry]);
        let export_state = ExportState::new(vec![bridge_container]);

        let next_vk = VerifyingKey::default();
        let moho_state = MohoState::new(inner_state_commitment, next_vk, export_state);

        let moho_state_commitment = moho_state.compute_commitment();

        let proven_ref =
            StateRefAttestation::new(StateReference::new([1u8; 32]), moho_state_commitment);

        let moho_attestation = MohoAttestation::new(genesis_ref.clone(), proven_ref);

        let moho_attestation_bytes = borsh::to_vec(&moho_attestation).unwrap();
        let public_values = PublicValues::new(moho_attestation_bytes);
        let proof_receipt = ProofReceipt::new(Proof::default(), public_values);

        BridgeCounterProofInput {
            moho_recursive_proof: proof_receipt,
            moho_state,
            anchor_state,
            claim_entry_id,
        }
    }

    #[test]
    fn test_process_bridge_counter_proof() {
        let deposit_idx: u32 = 42;
        let operator_idx: u32 = 7;
        let claim_entry_id: u32 = 1;

        let operator_claim = gen_mock_operator_claim_unlock(deposit_idx, operator_idx);
        let operator_claim_payload = borsh::to_vec(&operator_claim).unwrap();
        let genesis_ref = gen_mock_genesis_commitment();

        let mut header_verification_state = HeaderVerificationState::default();
        header_verification_state.total_accumulated_pow =
            BtcWork::from(Work::from_le_bytes([42u8; 32]));
        let expected_tip_total_work: Work =
            header_verification_state.total_accumulated_pow.clone().into();

        let input = gen_mock_bridge_counter_proof_input(
            &genesis_ref,
            operator_claim_payload,
            claim_entry_id,
            header_verification_state,
        );

        let output = process_bridge_counter_proof(&input, &genesis_ref);

        assert_eq!(output.deposit_idx, deposit_idx);
        assert_eq!(output.operator_idx, operator_idx);
        assert_eq!(output.tip_total_work, expected_tip_total_work);
    }
}
