use moho_runtime_interface::MohoProgram;
use moho_types::{ExportState, InnerStateCommitment, InnerVerificationKey, StateReference};
use strata_asm_common::{AnchorState, AsmLog};
use strata_asm_stf::{StrataAsmSpec, asm_stf};
use strata_primitives::hash::compute_borsh_hash;

use crate::input::AsmStepInput;

#[derive(Debug)]
pub struct AsmStfProgram;

impl MohoProgram for AsmStfProgram {
    type State = AnchorState;

    type StepInput = AsmStepInput;

    type StepOutput = Vec<AsmLog>;

    fn compute_input_reference(input: &AsmStepInput) -> StateReference {
        input.compute_ref()
    }

    fn extract_prev_reference(input: &Self::StepInput) -> StateReference {
        input.compute_prev_ref()
    }

    fn compute_state_commitment(state: &AnchorState) -> InnerStateCommitment {
        InnerStateCommitment::new(compute_borsh_hash(state).into())
    }

    fn process_transition(
        pre_state: &AnchorState,
        inp: &AsmStepInput,
    ) -> (AnchorState, Vec<AsmLog>) {
        asm_stf::<StrataAsmSpec>(pre_state, &inp.block.0, &inp.aux_bundle).unwrap()
    }

    fn extract_next_vk(output: &Self::StepOutput) -> Option<InnerVerificationKey> {
        // Iterate through each AsmLog; if we find an AsmStfUpdate, grab its vk and return it.
        output.iter().find_map(|log| {
            if let AsmLog::AsmStfUpdate(update) = log {
                Some(update.new_vk.clone())
            } else {
                None
            }
        })
    }

    fn update_export_state(export_state: &mut ExportState, output: &Self::StepOutput) {
        // Iterate through each AsmLog; if we find an NewExportEntry, add it to ExportState
        for log in output {
            if let AsmLog::NewExportEntry(export) = log {
                export_state.add_entry(export.container_id, export.entry_data.clone());
            }
        }
    }
}
