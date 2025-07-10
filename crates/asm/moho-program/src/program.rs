use moho_runtime_interface::MohoProgram;
use moho_types::{InnerStateCommitment, StateReference};
use strata_asm_common::{AnchorState, Log};
use strata_asm_stf::{StrataAsmSpec, asm_stf};
use strata_primitives::hash::compute_borsh_hash;

use crate::input::AsmStepInput;

#[derive(Debug)]
pub struct AsmStfProgram;

impl MohoProgram for AsmStfProgram {
    type State = AnchorState;

    type StepInput = AsmStepInput;

    type StepOutput = Vec<Log>;

    fn compute_input_reference(input: &AsmStepInput) -> StateReference {
        input.compute_ref()
    }

    fn extract_prev_reference(input: &Self::StepInput) -> StateReference {
        input.compute_prev_ref()
    }

    fn compute_state_commitment(state: &AnchorState) -> InnerStateCommitment {
        InnerStateCommitment::new(compute_borsh_hash(state).into())
    }

    fn process_transition(pre_state: &AnchorState, inp: &AsmStepInput) -> (AnchorState, Vec<Log>) {
        asm_stf::<StrataAsmSpec>(pre_state, &inp.block.0, &inp.aux_bundle).unwrap()
    }

    fn extract_next_vk(_output: &Self::StepOutput) -> moho_types::InnerVerificationKey {
        todo!()
    }

    fn extract_export_state(_state: &Self::StepOutput) -> moho_types::ExportState {
        todo!()
    }
}
