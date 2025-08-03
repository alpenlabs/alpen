use moho_runtime_interface::MohoProgram;
use moho_types::{ExportState, InnerStateCommitment, InnerVerificationKey, StateReference};
use strata_asm_common::{AnchorState, AsmSpec};
use strata_asm_logs::{AsmStfUpdate, NewExportEntry};
use strata_asm_spec::StrataAsmSpec;
use strata_asm_stf::{AsmStfInput, AsmStfOutput, asm_stf, group_txs_by_subprotocol};
use strata_primitives::hash::compute_borsh_hash;

use crate::input::AsmStepInput;

#[derive(Debug)]
pub struct AsmStfProgram;

impl MohoProgram for AsmStfProgram {
    type State = AnchorState;

    type StepInput = AsmStepInput;

    type StepOutput = AsmStfOutput;

    fn compute_input_reference(input: &AsmStepInput) -> StateReference {
        input.compute_ref()
    }

    fn extract_prev_reference(input: &Self::StepInput) -> StateReference {
        input.compute_prev_ref()
    }

    fn compute_state_commitment(state: &AnchorState) -> InnerStateCommitment {
        InnerStateCommitment::new(compute_borsh_hash(state).into())
    }

    fn process_transition(pre_state: &AnchorState, input: &AsmStepInput) -> AsmStfOutput {
        // 1. Validate the input
        assert!(input.validate_block());

        let protocol_txs =
            group_txs_by_subprotocol(StrataAsmSpec::MAGIC_BYTES, &input.block.0.txdata);

        let stf_input = AsmStfInput {
            protocol_txs,
            header: &input.block.0.header,
            aux_input: &input.aux_bundle,
        };

        asm_stf::<StrataAsmSpec>(pre_state, stf_input).unwrap()
    }

    fn extract_post_state(output: &Self::StepOutput) -> &Self::State {
        &output.state
    }

    fn extract_next_vk(output: &Self::StepOutput) -> Option<InnerVerificationKey> {
        // Iterate through each AsmLog; if we find an AsmStfUpdate, grab its vk and return it.
        output.logs.iter().find_map(|log| {
            log.try_into_log::<AsmStfUpdate>()
                .ok()
                .map(|update| update.new_vk.clone())
        })
    }

    fn compute_export_state(export_state: ExportState, output: &Self::StepOutput) -> ExportState {
        // Iterate through each AsmLog; if we find an NewExportEntry, add it to ExportState
        let mut new_export_state = export_state;
        for log in &output.logs {
            if let Ok(export) = log.try_into_log::<NewExportEntry>() {
                new_export_state.add_entry(export.container_id, export.entry_data.clone());
            }
        }
        new_export_state
    }
}
