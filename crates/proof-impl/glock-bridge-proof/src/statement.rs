use moho_types::{MerkleProof, MohoAttestation, MohoState};
use strata_asm_proto_bridge_v1::OperatorClaimUnlock;

#[derive(Debug, Clone, Copy, Default)]
pub struct BridgeProofPublicOutput {
    pub tip_total_work: u64,
    pub deposit_idx: u32,
    pub operator_idx: u32,
}

pub(crate) type Groth16Proof = Vec<u8>;
const BRIGE_ID: u16 = 1;

#[derive(Debug)]
pub struct BridgeProofInput {
    pub moho_state: MohoState,
    pub moho_recursive_proof: Groth16Proof,
    pub claim: OperatorClaimUnlock,
    pub claim_inclusion_proof: MerkleProof,
}

pub fn process_bridge_proof(input: BridgeProofInput) -> BridgeProofPublicOutput {
    let moho_attestation = verify_and_extract_public_params(input.moho_recursive_proof);
    assert_eq!(
        &input.moho_state.compute_commitment(),
        moho_attestation.proven().commitment()
    );

    // TODO: assert
    let export_state = input.moho_state.export_state();
    let _bridge_container = export_state
        .containers()
        .iter()
        .find(|x| x.container_id() == BRIGE_ID)
        .expect("Could not find bridge container in moho state");

    // TODO: assert claim in bridge_container

    BridgeProofPublicOutput {
        deposit_idx: input.claim.deposit_idx,
        operator_idx: input.claim.operator_idx,
        tip_total_work: 0,
    }
}

fn verify_and_extract_public_params(proof: Groth16Proof) -> MohoAttestation {
    get_mock_moho_attestation()
}

fn get_mock_moho_attestation() -> MohoAttestation {
    todo!()
}

fn get_mock_claim() -> OperatorClaimUnlock {
    todo!()
}

fn get_mock_bridge_proof_input() -> BridgeProofInput {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_params_resolves_bridge_input() {
        let input = get_mock_bridge_proof_input();
        let output = process_bridge_proof(input);

        assert_eq!(output.deposit_idx, input.claim.deposit_idx);
        assert_eq!(output.operator_idx, input.claim.operator_idx);
    }
}
