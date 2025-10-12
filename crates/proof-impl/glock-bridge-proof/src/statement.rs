use bitcoin::Work;
use moho_types::{MohoAttestation, MohoState, StateRefAttestation};
use strata_asm_common::AnchorState;
use strata_asm_moho_program_impl::{AsmStfProgram, MohoProgram};
use strata_asm_proto_bridge_v1::OperatorClaimUnlock;
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::proof::RollupVerifyingKey;
use zkaleido::ProofReceipt;

#[derive(Debug, Clone, Copy)]
pub struct BridgeProofPublicOutput {
    pub tip_total_work: Work,
    pub deposit_idx: u32,
    pub operator_idx: u32,
}

const BRIDGE_ID: u16 = 0;
const MOHO_VK: RollupVerifyingKey = RollupVerifyingKey::NativeVerifyingKey;

#[derive(Debug)]
pub struct BridgeProofInput {
    pub moho_recursive_proof: ProofReceipt,
    pub moho_state: MohoState,
    pub anchor_state: AnchorState,
    pub claim_entry_id: u32,
}

pub fn process_bridge_proof(
    input: &BridgeProofInput,
    genesis_moho: &StateRefAttestation,
) -> BridgeProofPublicOutput {
    let proof = &input.moho_recursive_proof;

    // Verify the recursive Moho proof and reconstruct its public parameters
    verify_rollup_groth16_proof_receipt(proof, &MOHO_VK).expect("Failed to verify groth16 proof");

    let moho_attestation = borsh::from_slice::<MohoAttestation>(proof.public_values().as_bytes())
        .expect("Failed to decode public params");

    // Ensure the proof chain starts from the canonical genesis state
    assert_eq!(moho_attestation.genesis(), genesis_moho, "Genesis mismatch");

    // Verify that the provided Moho state matches the committed attestation
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

    BridgeProofPublicOutput {
        deposit_idx: operator_claim.deposit_idx,
        operator_idx: operator_claim.operator_idx,
        tip_total_work,
    }
}
